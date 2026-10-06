//! Reads the title ID of a PS2 ISO or a PS1 CUE/BIN from the SYSTEM.CNF file on the disc,
//! the same way the console does: `BOOT2 = cdrom0:\SLUS_202.12;1` (PS2) or `BOOT = cdrom:\SCUS_949.00;1` (PS1).

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// ISO9660 logical sector size
const SECTOR: usize = 2048;
/// The primary volume descriptor is at sector 16
const PVD_SECTOR: u64 = 16;
/// Directories larger than this are not read (a disc root has a few sectors at most)
const MAX_DIR_SIZE: u32 = 64 * 1024;

/// A disc image with `sector_size`-byte sectors whose 2048 bytes of data start at `data_offset`
pub struct Disc {
    file: File,
    sector_size: u64,
    data_offset: u64,
}

impl Disc {
    /// An ISO image (2048-byte sectors)
    pub fn iso(path: &Path) -> io::Result<Disc> {
        Ok(Disc {
            file: File::open(path)?,
            sector_size: SECTOR as u64,
            data_offset: 0,
        })
    }

    /// A raw track image, like the first track of a CUE/BIN
    pub fn raw(path: &Path, sector_size: u64, data_offset: u64) -> io::Result<Disc> {
        Ok(Disc {
            file: File::open(path)?,
            sector_size,
            data_offset,
        })
    }

    fn read_sector(&mut self, lba: u64) -> io::Result<[u8; SECTOR]> {
        let mut buf = [0u8; SECTOR];
        self.file
            .seek(SeekFrom::Start(lba * self.sector_size + self.data_offset))?;
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Reads `size` bytes of a file or directory starting at sector `lba`
    fn read_extent(&mut self, lba: u32, size: u32) -> io::Result<Vec<u8>> {
        let mut data = Vec::with_capacity(size as usize);
        let sectors = (size as usize).div_ceil(SECTOR);
        for i in 0..sectors {
            data.extend_from_slice(&self.read_sector(lba as u64 + i as u64)?);
        }
        data.truncate(size as usize);
        Ok(data)
    }

    /// Lists the root directory: (name without the ";1" version, sector, size) of each file
    pub fn root_files(&mut self) -> io::Result<Vec<(String, u32, u32)>> {
        let pvd = self.read_sector(PVD_SECTOR)?;
        if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an ISO9660 disc",
            ));
        }
        // The root directory record is at offset 156 of the primary volume descriptor
        let root = &pvd[156..190];
        let (lba, size) = (le32(&root[2..6]), le32(&root[10..14]));
        let dir = self.read_extent(lba, size.min(MAX_DIR_SIZE))?;

        let mut files = Vec::new();
        let mut pos = 0;
        while pos < dir.len() {
            let len = dir[pos] as usize;
            if len == 0 {
                // Records don't cross sectors: the rest of this sector is padding
                pos = (pos / SECTOR + 1) * SECTOR;
                continue;
            }
            if pos + len > dir.len() || len < 34 {
                break;
            }
            let rec = &dir[pos..pos + len];
            let name_len = rec[32] as usize;
            if 33 + name_len <= len && rec[25] & 2 == 0 {
                // Not a directory
                let name = String::from_utf8_lossy(&rec[33..33 + name_len]);
                let name = name.split(';').next().unwrap_or("").to_string();
                files.push((name, le32(&rec[2..6]), le32(&rec[10..14])));
            }
            pos += len;
        }
        Ok(files)
    }

    /// Reads a file of the root directory, matching its name without case
    pub fn read_root_file(&mut self, name: &str) -> io::Result<Option<Vec<u8>>> {
        for (file, lba, size) in self.root_files()? {
            if file.eq_ignore_ascii_case(name) {
                return Ok(Some(self.read_extent(lba, size.min(MAX_DIR_SIZE))?));
            }
        }
        Ok(None)
    }

    /// The title ID from SYSTEM.CNF, or from a root file named like one (early PS1 discs without SYSTEM.CNF)
    pub fn title_id(&mut self) -> io::Result<Option<String>> {
        if let Some(cnf) = self.read_root_file("SYSTEM.CNF")?
            && let Some(id) = boot_id(&String::from_utf8_lossy(&cnf))
        {
            return Ok(Some(id));
        }
        Ok(self
            .root_files()?
            .iter()
            .find_map(|(name, _, _)| parse_id(name)))
    }
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// The title ID of the BOOT2 (PS2) or BOOT (PS1) line of a SYSTEM.CNF
pub fn boot_id(cnf: &str) -> Option<String> {
    for line in cnf.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !key.eq_ignore_ascii_case("BOOT2") && !key.eq_ignore_ascii_case("BOOT") {
            continue;
        }
        // cdrom0:\SLUS_202.12;1, cdrom:\DIR\SCUS_949.00;1 or cdrom:SLUS_005.94;1
        let value = value.trim();
        let file = value.rsplit(['\\', '/', ':']).next().unwrap_or(value);
        let file = file.split(';').next().unwrap_or(file);
        if let Some(id) = parse_id(file) {
            return Some(id);
        }
    }
    None
}

/// Looks for a title ID at the start of `s`, in the `SLUS_202.12`/`SLUS-202.12` or `SLUS-20212` forms,
/// and returns it as `SLUS_202.12` (OPL's form)
pub fn parse_id(s: &str) -> Option<String> {
    let b = s.as_bytes();
    if b.len() < 10 || !b[..4].iter().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let digits =
        |r: std::ops::Range<usize>| b.len() >= r.end && b[r].iter().all(|c| c.is_ascii_digit());
    if (b[4] == b'_' || b[4] == b'-') && digits(5..8) && b.get(8) == Some(&b'.') && digits(9..11) {
        return Some(format!("{}_{}.{}", &s[..4], &s[5..8], &s[9..11]));
    }
    if b[4] == b'-' && digits(5..10) {
        return Some(format!("{}_{}.{}", &s[..4], &s[5..8], &s[8..10]));
    }
    None
}

/// Searches the whole string for a title ID
pub fn find_id(s: &str) -> Option<String> {
    (0..s.len())
        .filter(|&i| s.is_char_boundary(i))
        .find_map(|i| parse_id(&s[i..]))
}

/// The first track of a CUE sheet: its BIN file and its sector layout
pub struct CueTrack {
    pub bin: PathBuf,
    pub sector_size: u64,
    pub data_offset: u64,
}

/// Reads the first FILE and TRACK lines of a CUE sheet. The BIN file is found without case,
/// since CUE sheets made on Windows don't always match the file names
pub fn read_cue(cue: &Path) -> io::Result<CueTrack> {
    let text = std::fs::read_to_string(cue)?;
    let dir = cue.parent().unwrap_or(Path::new("."));
    let mut bin = None;
    let mut mode = None;
    for line in text.lines() {
        let line = line.trim();
        let upper = line.to_ascii_uppercase();
        if bin.is_none() && upper.starts_with("FILE") {
            // FILE "name.bin" BINARY, or FILE name.bin BINARY
            let rest = line[4..].trim();
            let name = if let Some(quoted) = rest.strip_prefix('"') {
                quoted.split('"').next().unwrap_or("")
            } else {
                rest.split_whitespace().next().unwrap_or("")
            };
            bin = Some(name.to_string());
        } else if mode.is_none() && upper.starts_with("TRACK") {
            mode = upper.split_whitespace().nth(2).map(str::to_string);
        }
    }
    let bin = bin.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "no FILE line in the CUE sheet")
    })?;
    let (sector_size, data_offset) = match mode.as_deref() {
        Some("MODE1/2048") | Some("MODE2/2048") => (2048, 0),
        Some("MODE1/2352") => (2352, 16),
        Some("MODE2/2336") => (2336, 8),
        _ => (2352, 24), // MODE2/2352, what PS1 discs use
    };
    Ok(CueTrack {
        bin: find_file(dir, &bin)?,
        sector_size,
        data_offset,
    })
}

/// Finds `name` in `dir`, ignoring case if the exact name doesn't exist
fn find_file(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Ok(exact);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
        {
            return Ok(entry.path());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("{name} not found"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(parse_id("SLUS_202.12").as_deref(), Some("SLUS_202.12"));
        assert_eq!(
            parse_id("SLUS-202.12.Game.iso").as_deref(),
            Some("SLUS_202.12")
        );
        assert_eq!(parse_id("SCUS-94900").as_deref(), Some("SCUS_949.00"));
        assert_eq!(parse_id("slus_202.12"), None);
        assert_eq!(parse_id("SLUS_20"), None);
        assert_eq!(
            find_id("Crash Bandicoot [SCUS-94900]").as_deref(),
            Some("SCUS_949.00")
        );
    }

    #[test]
    fn system_cnf() {
        assert_eq!(
            boot_id("BOOT2 = cdrom0:\\SLUS_202.12;1\r\nVER = 1.00\r\n").as_deref(),
            Some("SLUS_202.12")
        );
        assert_eq!(
            boot_id("BOOT=cdrom:\\SCUS_949.00;1\nTCB=4\n").as_deref(),
            Some("SCUS_949.00")
        );
        assert_eq!(
            boot_id("BOOT = cdrom:SLUS_005.94;1").as_deref(),
            Some("SLUS_005.94")
        );
        assert_eq!(
            boot_id("BOOT = cdrom:\\GAME\\SLPS_000.01;1").as_deref(),
            Some("SLPS_000.01")
        );
        assert_eq!(boot_id("VMODE = NTSC"), None);
    }

    /// A minimal ISO9660 image: the volume descriptors, a root directory and one file in it
    pub(crate) fn iso_image(file: &str, data: &[u8]) -> Vec<u8> {
        let mut img = vec![0u8; SECTOR * 20];
        let record = |lba: u32, size: u32, flags: u8, name: &[u8]| {
            let len = 33 + name.len() + (name.len() + 1) % 2;
            let mut r = vec![0u8; len];
            r[0] = len as u8;
            r[2..6].copy_from_slice(&lba.to_le_bytes());
            r[10..14].copy_from_slice(&size.to_le_bytes());
            r[25] = flags;
            r[32] = name.len() as u8;
            r[33..33 + name.len()].copy_from_slice(name);
            r
        };
        let pvd = 16 * SECTOR;
        img[pvd] = 1;
        img[pvd + 1..pvd + 6].copy_from_slice(b"CD001");
        let root = record(18, SECTOR as u32, 2, &[0]);
        img[pvd + 156..pvd + 156 + root.len()].copy_from_slice(&root);
        img[17 * SECTOR] = 255;
        img[17 * SECTOR + 1..17 * SECTOR + 6].copy_from_slice(b"CD001");
        let mut dir = Vec::new();
        dir.extend(record(18, SECTOR as u32, 2, &[0]));
        dir.extend(record(18, SECTOR as u32, 2, &[1]));
        dir.extend(record(
            19,
            data.len() as u32,
            0,
            format!("{file};1").as_bytes(),
        ));
        img[18 * SECTOR..18 * SECTOR + dir.len()].copy_from_slice(&dir);
        img[19 * SECTOR..19 * SECTOR + data.len()].copy_from_slice(data);
        img
    }

    /// The image as a raw MODE2/2352 track: 24 bytes of sync, header and subheader before each sector
    pub(crate) fn raw_image(iso: &[u8]) -> Vec<u8> {
        iso.chunks(SECTOR)
            .flat_map(|s| [vec![0u8; 24], s.to_vec(), vec![0u8; 280]].concat())
            .collect()
    }

    #[test]
    fn disc_images() {
        let dir = std::env::temp_dir().join(format!("osdhub-manager-disc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let iso = dir.join("game.iso");
        std::fs::write(
            &iso,
            iso_image(
                "SYSTEM.CNF",
                b"BOOT2 = cdrom0:\\SLUS_202.12;1\r\nVER = 1.00\r\n",
            ),
        )
        .unwrap();
        assert_eq!(
            Disc::iso(&iso).unwrap().title_id().unwrap().as_deref(),
            Some("SLUS_202.12")
        );

        let bin = dir.join("Game (Track 1).bin");
        std::fs::write(
            &bin,
            raw_image(&iso_image(
                "SYSTEM.CNF",
                b"BOOT = cdrom:\\SCUS_949.00;1\nTCB = 4\n",
            )),
        )
        .unwrap();
        let cue = dir.join("game.cue");
        std::fs::write(
            &cue,
            "FILE \"game (track 1).BIN\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        let track = read_cue(&cue).unwrap();
        assert_eq!((track.sector_size, track.data_offset), (2352, 24));
        let mut disc = Disc::raw(&track.bin, track.sector_size, track.data_offset).unwrap();
        assert_eq!(disc.title_id().unwrap().as_deref(), Some("SCUS_949.00"));

        // Early PS1 discs without SYSTEM.CNF: the executable is named after the ID
        let early = dir.join("early.iso");
        std::fs::write(&early, iso_image("SLPS_000.01", b"PS-X EXE")).unwrap();
        assert_eq!(
            Disc::iso(&early).unwrap().title_id().unwrap().as_deref(),
            Some("SLPS_000.01")
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
