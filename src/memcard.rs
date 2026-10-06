//! PS2 memory card images, as MMCE devices (`MemoryCards/**/*.mcd`) and PCSX2 (`.ps2`) keep them: reads their
//! files, and replaces the contents of one, for OSDMenu's configuration (`SYS-CONF/OSDMENU.CNF`).
//!
//! The card is made of 512-byte pages, each followed by 16 spare bytes holding its ECC in the images that keep it
//! (8,650,752 bytes, like PCSX2's), or not (8 MB, like MMCE's). Two pages make a 1 KB cluster. The first page is the
//! superblock, which locates the FAT through a list of indirect FAT clusters. The FAT links the clusters of each
//! file and directory, numbered from the first allocatable cluster. A directory is a list of 512-byte entries
//! (two per cluster), starting with "." and "..": the "." entry of the root directory holds its number of entries.

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

const MAGIC: &[u8] = b"Sony PS2 Memory Card Format ";
const PAGE: usize = 512;
const SPARE: usize = 16;
const ENTRY: usize = 512;

/// FAT entries: a free cluster has the high bit clear, the last cluster of a chain is all ones,
/// and the others hold the next cluster with the high bit set
const FAT_FREE: u32 = 0x7FFF_FFFF;
const FAT_END: u32 = 0xFFFF_FFFF;
const FAT_USED: u32 = 0x8000_0000;

/// Entry modes
const MODE_FILE: u16 = 0x0010;
const MODE_DIR: u16 = 0x0020;
const MODE_EXISTS: u16 = 0x8000;
/// The modes the PS2 gives to new files and directories (readable, writable, executable and 0x0400, plus 0x0080
/// for files)
const MODE_NEW_FILE: u16 = 0x8497;
const MODE_NEW_DIR: u16 = 0x8427;

pub struct Card {
    /// The pages, without their spare bytes
    data: Vec<u8>,
    /// The spare bytes of each page, when the image keeps them
    spare: Option<Vec<[u8; SPARE]>>,
    /// Pages changed, whose ECC is computed again
    dirty: BTreeSet<usize>,
    pages_per_cluster: usize,
    clusters: u32,
    alloc_offset: u32,
    alloc_end: u32,
    root_cluster: u32,
    ifc: Vec<u32>,
}

/// A directory entry, and where it is: the cluster of its directory (numbered from the first allocatable one)
/// and its index in that cluster
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub mode: u16,
    pub length: u32,
    pub cluster: u32,
    location: (u32, usize),
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.mode & MODE_DIR != 0
    }

    pub fn is_file(&self) -> bool {
        self.mode & MODE_FILE != 0
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl Card {
    /// Reads a card image, with or without the spare bytes of its pages
    pub fn open(image: &[u8]) -> Result<Card, String> {
        if !image.starts_with(MAGIC) {
            return Err("not a PS2 memory card image".to_string());
        }
        let page_len = u16_at(image, 0x28) as usize;
        let pages_per_cluster = u16_at(image, 0x2A) as usize;
        let clusters = u32_at(image, 0x30);
        if page_len != PAGE || pages_per_cluster == 0 || pages_per_cluster > 8 {
            return Err(format!(
                "unsupported memory card ({page_len}-byte pages, {pages_per_cluster} per cluster)"
            ));
        }
        let pages = clusters as usize * pages_per_cluster;
        let (data, spare) = if image.len() == pages * PAGE {
            (image.to_vec(), None)
        } else if image.len() == pages * (PAGE + SPARE) {
            let mut data = Vec::with_capacity(pages * PAGE);
            let mut spare = Vec::with_capacity(pages);
            for page in image.as_chunks::<{ PAGE + SPARE }>().0 {
                data.extend_from_slice(&page[..PAGE]);
                spare.push(page[PAGE..].try_into().unwrap());
            }
            (data, Some(spare))
        } else {
            return Err(format!(
                "a memory card of {clusters} clusters can't be {} bytes",
                image.len()
            ));
        };
        let ifc = (0..32)
            .map(|i| u32_at(&data, 0x50 + i * 4))
            .take_while(|&c| c != 0 && c < clusters)
            .collect();
        let card = Card {
            pages_per_cluster,
            clusters,
            alloc_offset: u32_at(&data, 0x34),
            alloc_end: u32_at(&data, 0x38),
            root_cluster: u32_at(&data, 0x3C),
            ifc,
            data,
            spare,
            dirty: BTreeSet::new(),
        };
        if card.alloc_offset + card.alloc_end > clusters || card.ifc.is_empty() {
            return Err("the superblock of the memory card is damaged".to_string());
        }
        Ok(card)
    }

    /// The image, with the ECC of the pages changed computed again
    pub fn to_bytes(&self) -> Vec<u8> {
        let Some(spare) = &self.spare else {
            return self.data.clone();
        };
        let mut image = Vec::with_capacity(spare.len() * (PAGE + SPARE));
        for (index, page) in self.data.as_chunks::<PAGE>().0.iter().enumerate() {
            image.extend_from_slice(page);
            if self.dirty.contains(&index) {
                let mut new = [0u8; SPARE];
                let (eccs, _) = new.as_chunks_mut::<3>();
                for (chunk, ecc) in page.as_chunks::<128>().0.iter().zip(eccs) {
                    *ecc = ecc_of(chunk);
                }
                image.extend_from_slice(&new);
            } else {
                image.extend_from_slice(&spare[index]);
            }
        }
        image
    }

    fn cluster_size(&self) -> usize {
        self.pages_per_cluster * PAGE
    }

    /// A cluster by its number on the card
    fn cluster(&self, absolute: u32) -> Result<&[u8], String> {
        if absolute >= self.clusters {
            return Err(format!("cluster {absolute} is outside the memory card"));
        }
        let at = absolute as usize * self.cluster_size();
        Ok(&self.data[at..at + self.cluster_size()])
    }

    fn write(&mut self, absolute: u32, offset: usize, bytes: &[u8]) {
        let at = absolute as usize * self.cluster_size() + offset;
        self.data[at..at + bytes.len()].copy_from_slice(bytes);
        self.dirty
            .extend(at / PAGE..(at + bytes.len()).div_ceil(PAGE));
    }

    /// Where the FAT entry of an allocatable cluster is, as (FAT cluster on the card, offset)
    fn fat_location(&self, cluster: u32) -> Result<(u32, usize), String> {
        let per_cluster = (self.cluster_size() / 4) as u32;
        let fat_index = cluster / per_cluster;
        let ifc_index = (fat_index / per_cluster) as usize;
        let ifc = *self
            .ifc
            .get(ifc_index)
            .ok_or(format!("cluster {cluster} has no FAT entry"))?;
        let fat_cluster = u32_at(self.cluster(ifc)?, (fat_index % per_cluster) as usize * 4);
        self.cluster(fat_cluster)?;
        Ok((fat_cluster, (cluster % per_cluster) as usize * 4))
    }

    fn fat(&self, cluster: u32) -> Result<u32, String> {
        let (fat_cluster, offset) = self.fat_location(cluster)?;
        Ok(u32_at(self.cluster(fat_cluster)?, offset))
    }

    fn set_fat(&mut self, cluster: u32, value: u32) -> Result<(), String> {
        let (fat_cluster, offset) = self.fat_location(cluster)?;
        self.write(fat_cluster, offset, &value.to_le_bytes());
        Ok(())
    }

    /// The clusters of a file or directory, from its first one
    fn chain(&self, first: u32, count: usize) -> Result<Vec<u32>, String> {
        let mut chain = Vec::with_capacity(count);
        let mut cluster = first;
        while chain.len() < count {
            if cluster >= self.alloc_end || chain.contains(&cluster) {
                return Err(format!("broken cluster chain at cluster {cluster}"));
            }
            chain.push(cluster);
            let next = self.fat(cluster)?;
            if next == FAT_END || next & FAT_USED == 0 {
                break;
            }
            cluster = next & !FAT_USED;
        }
        if chain.len() < count {
            return Err(format!(
                "a chain from cluster {first} is shorter than its length"
            ));
        }
        Ok(chain)
    }

    fn entry_at(&self, dir_cluster: u32, index: usize) -> Result<Entry, String> {
        let raw =
            &self.cluster(self.alloc_offset + dir_cluster)?[index * ENTRY..(index + 1) * ENTRY];
        let name = &raw[0x40..0x60];
        let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
        Ok(Entry {
            name: String::from_utf8_lossy(name).into_owned(),
            mode: u16_at(raw, 0),
            length: u32_at(raw, 4),
            cluster: u32_at(raw, 0x10),
            location: (dir_cluster, index),
        })
    }

    fn entries_per_cluster(&self) -> usize {
        self.cluster_size() / ENTRY
    }

    /// The entries of a directory, without "." and ".." nor the deleted ones
    pub fn list(&self, dir: &Entry) -> Result<Vec<Entry>, String> {
        if !dir.is_dir() {
            return Err(format!("{} isn't a directory", dir.name));
        }
        let per = self.entries_per_cluster();
        let count = dir.length as usize;
        let chain = self.chain(dir.cluster, count.div_ceil(per))?;
        let mut entries = Vec::new();
        for i in 2..count {
            let entry = self.entry_at(chain[i / per], i % per)?;
            if entry.mode & MODE_EXISTS != 0 {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    pub fn root(&self) -> Result<Entry, String> {
        let mut root = self.entry_at(self.root_cluster, 0)?;
        root.name = "/".to_string();
        root.cluster = self.root_cluster;
        Ok(root)
    }

    /// The entry at a path like `SYS-CONF/OSDMENU.CNF`, matching the names without case as the PS2 does
    pub fn find(&self, path: &str) -> Result<Entry, String> {
        let mut entry = self.root()?;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            entry = self
                .list(&entry)?
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(part))
                .ok_or(format!("{path} isn't on the memory card"))?;
        }
        Ok(entry)
    }

    pub fn exists(&self, path: &str) -> bool {
        self.find(path).is_ok()
    }

    pub fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let entry = self.find(path)?;
        if !entry.is_file() {
            return Err(format!("{path} isn't a file"));
        }
        let size = self.cluster_size();
        let length = entry.length as usize;
        let mut data = Vec::with_capacity(length);
        for cluster in self.chain(entry.cluster, length.div_ceil(size))? {
            data.extend_from_slice(self.cluster(self.alloc_offset + cluster)?);
        }
        data.truncate(length);
        Ok(data)
    }

    /// Writes the file at `path`, replacing its contents or creating it, with the directories leading to it
    pub fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), String> {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let Some((name, dirs)) = parts.split_last() else {
            return Err("no file name".to_string());
        };
        let mut dir = self.root()?;
        for part in dirs {
            let found = self
                .list(&dir)?
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(part));
            dir = match found {
                Some(entry) if entry.is_dir() => entry,
                Some(_) => return Err(format!("{part} isn't a directory")),
                None => self.make_dir(&dir, part)?,
            };
        }
        let found = self
            .list(&dir)?
            .into_iter()
            .find(|e| e.name.eq_ignore_ascii_case(name));
        let (entry, old) = match found {
            Some(entry) if entry.is_file() => {
                let size = self.cluster_size();
                let count = (entry.length as usize).div_ceil(size);
                let old = if count > 0 {
                    self.chain(entry.cluster, count)?
                } else {
                    Vec::new()
                };
                (entry, old)
            }
            Some(_) => return Err(format!("{path} isn't a file")),
            None => (
                self.add_entry(&dir, name, MODE_NEW_FILE, 0, FAT_END)?.0,
                Vec::new(),
            ),
        };
        let chain = self.write_chain(&old, contents)?;

        // The entry's first cluster, length and modification time
        let (dir_cluster, index) = entry.location;
        let at = index * ENTRY;
        let absolute = self.alloc_offset + dir_cluster;
        self.write(absolute, at + 0x04, &(contents.len() as u32).to_le_bytes());
        self.write(absolute, at + 0x10, &chain[0].to_le_bytes());
        self.write(absolute, at + 0x18, &now());
        Ok(())
    }

    /// Free clusters, other than `taken`
    fn allocate(&self, count: usize, taken: &[u32]) -> Result<Vec<u32>, String> {
        let mut free = Vec::with_capacity(count);
        for cluster in 0..self.alloc_end {
            if free.len() == count {
                break;
            }
            if self.fat(cluster)? & FAT_USED == 0 && !taken.contains(&cluster) {
                free.push(cluster);
            }
        }
        if free.len() < count {
            return Err("the memory card is full".to_string());
        }
        Ok(free)
    }

    /// Writes `contents` in the clusters of `old`, taking free ones when it needs more and freeing the ones left
    fn write_chain(&mut self, old: &[u32], contents: &[u8]) -> Result<Vec<u32>, String> {
        let size = self.cluster_size();
        let needed = contents.len().div_ceil(size).max(1);
        let mut chain: Vec<u32> = old.iter().copied().take(needed).collect();
        if chain.len() < needed {
            let free = self.allocate(needed - chain.len(), old)?;
            chain.extend(free);
        }
        for (i, &cluster) in chain.iter().enumerate() {
            let mut bytes = contents.get(i * size..).unwrap_or(&[]).to_vec();
            bytes.resize(size, 0);
            self.write(self.alloc_offset + cluster, 0, &bytes);
            let next = chain.get(i + 1).map_or(FAT_END, |next| FAT_USED | next);
            self.set_fat(cluster, next)?;
        }
        for &cluster in &old[needed.min(old.len())..] {
            self.set_fat(cluster, FAT_FREE)?;
        }
        Ok(chain)
    }

    /// Adds an entry to a directory, in the place of a deleted one or after the others, returning it and its index
    fn add_entry(
        &mut self,
        dir: &Entry,
        name: &str,
        mode: u16,
        length: u32,
        cluster: u32,
    ) -> Result<(Entry, usize), String> {
        if name.is_empty() || name.len() > 31 || !name.is_ascii() || name.contains(['/', '\\']) {
            return Err(format!("{name} can't be a memory card file name"));
        }
        let per = self.entries_per_cluster();
        let count = dir.length as usize;
        let mut chain = self.chain(dir.cluster, count.div_ceil(per))?;
        let mut index = count;
        for i in 2..count {
            if self.entry_at(chain[i / per], i % per)?.mode & MODE_EXISTS == 0 {
                index = i;
                break;
            }
        }
        let now = now();
        if index == count {
            if index / per >= chain.len() {
                let [new] = self.allocate(1, &[])?[..] else {
                    unreachable!()
                };
                self.set_fat(*chain.last().unwrap(), FAT_USED | new)?;
                self.set_fat(new, FAT_END)?;
                self.write(self.alloc_offset + new, 0, &vec![0; self.cluster_size()]);
                chain.push(new);
            }
            // The number of entries is in the directory's entry ("." for the root)
            let (dir_cluster, dir_index) = dir.location;
            self.write(
                self.alloc_offset + dir_cluster,
                dir_index * ENTRY + 0x04,
                &(count as u32 + 1).to_le_bytes(),
            );
        }
        let (dir_cluster, dir_index) = dir.location;
        self.write(
            self.alloc_offset + dir_cluster,
            dir_index * ENTRY + 0x18,
            &now,
        );

        let location = (chain[index / per], index % per);
        let mut raw = vec![0u8; ENTRY];
        raw[0..2].copy_from_slice(&mode.to_le_bytes());
        raw[4..8].copy_from_slice(&length.to_le_bytes());
        raw[0x08..0x10].copy_from_slice(&now);
        raw[0x10..0x14].copy_from_slice(&cluster.to_le_bytes());
        raw[0x18..0x20].copy_from_slice(&now);
        raw[0x40..0x40 + name.len()].copy_from_slice(name.as_bytes());
        self.write(self.alloc_offset + location.0, location.1 * ENTRY, &raw);
        Ok((self.entry_at(location.0, location.1)?, index))
    }

    /// Makes a directory with its "." entry, which points to the parent directory and the directory's entry in it,
    /// and its ".." entry, as the PS2 does
    fn make_dir(&mut self, parent: &Entry, name: &str) -> Result<Entry, String> {
        let [cluster] = self.allocate(1, &[])?[..] else {
            unreachable!()
        };
        self.set_fat(cluster, FAT_END)?;
        let (entry, index) = self.add_entry(parent, name, MODE_NEW_DIR, 2, cluster)?;
        let now = now();
        let mut raw = vec![0u8; self.cluster_size()];
        for (i, (name, cluster, dirent)) in [(".", parent.cluster, index as u32), ("..", 0, 0)]
            .into_iter()
            .enumerate()
        {
            let at = i * ENTRY;
            raw[at..at + 2].copy_from_slice(&MODE_NEW_DIR.to_le_bytes());
            raw[at + 0x08..at + 0x10].copy_from_slice(&now);
            raw[at + 0x10..at + 0x14].copy_from_slice(&cluster.to_le_bytes());
            raw[at + 0x14..at + 0x18].copy_from_slice(&dirent.to_le_bytes());
            raw[at + 0x18..at + 0x20].copy_from_slice(&now);
            raw[at + 0x40..at + 0x40 + name.len()].copy_from_slice(name.as_bytes());
        }
        self.write(self.alloc_offset + cluster, 0, &raw);
        Ok(entry)
    }

    /// Checks that every directory and file can be read, and that no cluster belongs to two of them
    pub fn check(&self) -> Result<(), String> {
        let mut used = BTreeSet::new();
        let mut pending = vec![(self.root()?, "/".to_string())];
        while let Some((dir, path)) = pending.pop() {
            let per = self.entries_per_cluster();
            for cluster in self.chain(dir.cluster, (dir.length as usize).div_ceil(per))? {
                if !used.insert(cluster) {
                    return Err(format!("{path} shares cluster {cluster}"));
                }
            }
            for entry in self.list(&dir)? {
                let entry_path = format!("{}{}", path, entry.name);
                if entry.is_dir() {
                    pending.push((entry, format!("{entry_path}/")));
                } else if entry.length > 0 {
                    let count = (entry.length as usize).div_ceil(self.cluster_size());
                    for cluster in self.chain(entry.cluster, count)? {
                        if !used.insert(cluster) {
                            return Err(format!("{entry_path} shares cluster {cluster}"));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// The ECC of 128 bytes of a page: a Hamming code made of the column parity and two line parities
fn ecc_of(chunk: &[u8]) -> [u8; 3] {
    const COLUMN_MASKS: [u8; 7] = [0x55, 0x33, 0x0F, 0x00, 0xAA, 0xCC, 0xF0];
    let parity = |b: u8| (b.count_ones() & 1) as u8;
    let mut column = 0x77u8;
    let mut line0 = 0x7Fu8;
    let mut line1 = 0x7Fu8;
    for (i, &b) in chunk.iter().enumerate() {
        for (bit, mask) in COLUMN_MASKS.iter().enumerate() {
            column ^= parity(b & mask) << bit;
        }
        if parity(b) == 1 {
            line0 ^= !(i as u8);
            line1 ^= i as u8;
        }
    }
    [column, line0 & 0x7F, line1]
}

/// The current time as the PS2 keeps it: Japan's time, as (unused, seconds, minutes, hours, day, month, year)
fn now() -> [u8; 8] {
    let (year, month, day, hours, minutes, seconds) = date_time(9);
    let [y0, y1] = year.to_le_bytes();
    [0, seconds, minutes, hours, day, month, y0, y1]
}

/// The current date and time, `offset` hours from UTC, as (year, month, day, hours, minutes, seconds)
pub fn date_time(offset: i64) -> (u16, u8, u8, u8, u8, u8) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
        + offset * 3600;
    let days = seconds.div_euclid(86400);
    let time = seconds.rem_euclid(86400);
    // Days to a civil date (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as u16;
    (
        year,
        month,
        day,
        (time / 3600) as u8,
        (time / 60 % 60) as u8,
        (time % 60) as u8,
    )
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A formatted 8 MB card like the PS2 makes, with SYS-CONF/OSDMENU.CNF and a save directory,
    /// built by hand to test the reader against the layout described above
    pub fn card_image(cnf: &[u8], ecc: bool) -> Vec<u8> {
        let (clusters, alloc_offset, alloc_end) = (8192u32, 41u32, 8135u32);
        let mut data = vec![0xFFu8; clusters as usize * 1024];
        data[..MAGIC.len()].copy_from_slice(MAGIC);
        data[28..35].copy_from_slice(b"1.2.0.0");
        let put16 =
            |d: &mut Vec<u8>, at: usize, v: u16| d[at..at + 2].copy_from_slice(&v.to_le_bytes());
        let put32 =
            |d: &mut Vec<u8>, at: usize, v: u32| d[at..at + 4].copy_from_slice(&v.to_le_bytes());
        put16(&mut data, 0x28, 512);
        put16(&mut data, 0x2A, 2);
        put16(&mut data, 0x2C, 16);
        put32(&mut data, 0x30, clusters);
        put32(&mut data, 0x34, alloc_offset);
        put32(&mut data, 0x38, alloc_end);
        put32(&mut data, 0x3C, 0);
        for i in 0..32 {
            put32(&mut data, 0x50 + i * 4, if i == 0 { 8 } else { 0 });
        }
        // The indirect FAT cluster 8 points to the FAT clusters 9 to 40, all free
        for i in 0..32u32 {
            put32(&mut data, 8 * 1024 + i as usize * 4, 9 + i);
        }
        for cluster in 0..alloc_end {
            let at = (9 + cluster / 256) as usize * 1024 + (cluster % 256) as usize * 4;
            put32(&mut data, at, FAT_FREE);
        }
        let fat = |d: &mut Vec<u8>, cluster: u32, v: u32| {
            let at = (9 + cluster / 256) as usize * 1024 + (cluster % 256) as usize * 4;
            d[at..at + 4].copy_from_slice(&v.to_le_bytes());
        };
        let entry = |d: &mut Vec<u8>,
                     cluster: u32,
                     index: usize,
                     mode: u16,
                     length: u32,
                     first: u32,
                     name: &str| {
            let at = (alloc_offset + cluster) as usize * 1024 + index * 512;
            d[at..at + 512].fill(0);
            d[at..at + 2].copy_from_slice(&mode.to_le_bytes());
            d[at + 4..at + 8].copy_from_slice(&length.to_le_bytes());
            d[at + 0x10..at + 0x14].copy_from_slice(&first.to_le_bytes());
            d[at + 0x40..at + 0x40 + name.len()].copy_from_slice(name.as_bytes());
        };
        let dir = MODE_EXISTS | MODE_DIR | 0x0407;
        let file = MODE_EXISTS | MODE_FILE | 0x0417;
        // Root (cluster 0-1): ".", "..", SYS-CONF, BESLES-SAVE
        entry(&mut data, 0, 0, dir, 4, 0, ".");
        entry(&mut data, 0, 1, dir, 0, 0, "..");
        entry(&mut data, 1, 0, dir, 3, 2, "SYS-CONF");
        entry(&mut data, 1, 1, dir, 3, 4, "BESLES-SAVE");
        fat(&mut data, 0, FAT_USED | 1);
        fat(&mut data, 1, FAT_END);
        // SYS-CONF (clusters 2-3): ".", "..", OSDMENU.CNF in cluster 6 on
        entry(&mut data, 2, 0, dir, 3, 0, ".");
        entry(&mut data, 2, 1, dir, 0, 0, "..");
        entry(&mut data, 3, 0, file, cnf.len() as u32, 6, "OSDMENU.CNF");
        fat(&mut data, 2, FAT_USED | 3);
        fat(&mut data, 3, FAT_END);
        // BESLES-SAVE (clusters 4-5) with a 1500-byte save after the CNF
        entry(&mut data, 4, 0, dir, 3, 0, ".");
        entry(&mut data, 4, 1, dir, 0, 0, "..");
        let cnf_clusters = cnf.len().div_ceil(1024).max(1) as u32;
        let save = 6 + cnf_clusters;
        entry(&mut data, 5, 0, file, 1500, save, "SAVE.DAT");
        fat(&mut data, 4, FAT_USED | 5);
        fat(&mut data, 5, FAT_END);
        for i in 0..cnf_clusters {
            let cluster = 6 + i;
            let at = (alloc_offset + cluster) as usize * 1024;
            let part =
                &cnf[(i as usize * 1024).min(cnf.len())..((i as usize + 1) * 1024).min(cnf.len())];
            data[at..at + 1024].fill(0);
            data[at..at + part.len()].copy_from_slice(part);
            fat(
                &mut data,
                cluster,
                if i + 1 < cnf_clusters {
                    FAT_USED | (cluster + 1)
                } else {
                    FAT_END
                },
            );
        }
        for i in 0..2 {
            let at = (alloc_offset + save + i) as usize * 1024;
            data[at..at + 1024].fill(0x5A);
        }
        fat(&mut data, save, FAT_USED | (save + 1));
        fat(&mut data, save + 1, FAT_END);
        if !ecc {
            return data;
        }
        let mut image = Vec::new();
        for page in data.as_chunks::<512>().0 {
            image.extend_from_slice(page);
            for chunk in page.as_chunks::<128>().0 {
                image.extend_from_slice(&ecc_of(chunk));
            }
            image.extend_from_slice(&[0; 4]);
        }
        image
    }

    #[test]
    fn read_and_replace() {
        for ecc in [false, true] {
            let cnf = b"OSDSYS_menu_x = 320\ngames_covers = 1\n";
            let image = card_image(cnf, ecc);
            assert_eq!(image.len(), if ecc { 8_650_752 } else { 8_388_608 });
            let mut card = Card::open(&image).unwrap();
            card.check().unwrap();
            assert_eq!(card.read("SYS-CONF/OSDMENU.CNF").unwrap(), cnf);
            assert_eq!(card.read("/sys-conf/osdmenu.cnf").unwrap(), cnf);
            assert!(card.exists("BESLES-SAVE/SAVE.DAT"));
            assert!(!card.exists("SYS-CONF/NOPE.CNF"));
            // Unchanged, the image is the same
            assert_eq!(card.to_bytes(), image);

            // Growing to 3 clusters takes free ones, without touching the save
            let big: Vec<u8> = (0..2500).map(|i| b"abcdefghij\n"[i % 11]).collect();
            card.write_file("SYS-CONF/OSDMENU.CNF", &big).unwrap();
            let mut card = Card::open(&card.to_bytes()).unwrap();
            card.check().unwrap();
            assert_eq!(card.read("SYS-CONF/OSDMENU.CNF").unwrap(), big);
            assert_eq!(card.read("BESLES-SAVE/SAVE.DAT").unwrap(), vec![0x5A; 1500]);

            // Shrinking frees the clusters left
            card.write_file("SYS-CONF/OSDMENU.CNF", b"small\n").unwrap();
            let image = card.to_bytes();
            let card = Card::open(&image).unwrap();
            card.check().unwrap();
            assert_eq!(card.read("SYS-CONF/OSDMENU.CNF").unwrap(), b"small\n");
            let used = (0..card.alloc_end)
                .filter(|&c| card.fat(c).unwrap() & FAT_USED != 0)
                .count();
            assert_eq!(used, 2 + 2 + 2 + 1 + 2);

            // The ECC of every page is right
            if ecc {
                for page in image.as_chunks::<528>().0 {
                    for (i, chunk) in page[..512].as_chunks::<128>().0.iter().enumerate() {
                        assert_eq!(page[512 + i * 3..512 + i * 3 + 3], ecc_of(chunk));
                    }
                }
            }
        }
    }

    /// Replaces the CNF of a card image made by another tool, given in OSDHUB_MCD, to check it with that tool:
    /// `OSDHUB_MCD=card.ps2 cargo test external -- --ignored`
    #[test]
    #[ignore]
    fn external() {
        let path = std::env::var("OSDHUB_MCD").unwrap();
        let image = std::fs::read(&path).unwrap();
        let mut card = Card::open(&image).unwrap();
        card.check().unwrap();
        let mut cnf = card.read("SYS-CONF/OSDMENU.CNF").unwrap_or_default();
        println!("before: {}", String::from_utf8_lossy(&cnf));
        for i in 0..150 {
            cnf.extend_from_slice(format!("name_OSDSYS_ITEM_{i} = Entry {i}\n").as_bytes());
        }
        card.write_file("SYS-CONF/OSDMENU.CNF", &cnf).unwrap();
        std::fs::write(format!("{path}.new"), card.to_bytes()).unwrap();
        let card = Card::open(&std::fs::read(format!("{path}.new")).unwrap()).unwrap();
        card.check().unwrap();
        assert_eq!(card.read("SYS-CONF/OSDMENU.CNF").unwrap(), cnf);
    }

    #[test]
    fn create() {
        let image = card_image(b"x", false);
        let mut card = Card::open(&image).unwrap();
        // The root has 4 entries in its 2 clusters, so a new directory takes a third one
        card.write_file("NEW-DIR/FIRST.CNF", b"first").unwrap();
        card.write_file("NEW-DIR/SECOND.CNF", &[7; 3000]).unwrap();
        let mut card = Card::open(&card.to_bytes()).unwrap();
        card.check().unwrap();
        assert_eq!(card.read("NEW-DIR/FIRST.CNF").unwrap(), b"first");
        assert_eq!(card.read("NEW-DIR/SECOND.CNF").unwrap(), vec![7; 3000]);
        assert_eq!(card.root().unwrap().length, 5);
        let dir = card.find("NEW-DIR").unwrap();
        assert_eq!((dir.length, dir.mode), (4, MODE_NEW_DIR));
        // "." points to the root and to the directory's entry in it
        let dot = card.entry_at(dir.cluster, 0).unwrap();
        let raw = &card.cluster(card.alloc_offset + dir.cluster).unwrap()[..512];
        assert_eq!(
            (dot.name.as_str(), dot.cluster, u32_at(raw, 0x14)),
            (".", 0, 4)
        );
        assert_eq!(card.find("NEW-DIR/FIRST.CNF").unwrap().mode, MODE_NEW_FILE);
        assert!(card.write_file("SYS-CONF/OSDMENU.CNF/X", b"").is_err());
        assert!(
            card.write_file(&format!("{}/X", "N".repeat(40)), b"")
                .is_err()
        );
    }

    #[test]
    fn bad_images() {
        assert!(Card::open(b"not a card").is_err());
        let mut image = card_image(b"x", false);
        image.truncate(1_000_000);
        assert!(Card::open(&image).is_err());
    }

    #[test]
    fn time() {
        let [_, s, m, h, d, mo, y0, y1] = now();
        assert!(s < 60 && m < 60 && h < 24 && (1..=31).contains(&d) && (1..=12).contains(&mo));
        assert!(u16::from_le_bytes([y0, y1]) >= 2026);
    }
}

#[cfg(test)]
mod dump {
    use super::*;

    /// Lists a card image given in OSDHUB_MCD: `OSDHUB_MCD=card.mcd cargo test dump -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump() {
        let image = std::fs::read(std::env::var("OSDHUB_MCD").unwrap()).unwrap();
        let card = Card::open(&image).unwrap();
        println!("check: {:?}", card.check());
        let mut pending = vec![(card.root().unwrap(), String::new())];
        while let Some((dir, path)) = pending.pop() {
            for e in card.list(&dir).unwrap() {
                println!(
                    "{:04x} {:>8} {:>5} {path}{}",
                    e.mode, e.length, e.cluster, e.name
                );
                if e.is_dir() {
                    pending.push((e.clone(), format!("{path}{}/", e.name)));
                }
            }
        }
        let free = (0..card.alloc_end)
            .filter(|&c| card.fat(c).unwrap() & FAT_USED == 0)
            .count();
        println!("free clusters: {free}");
    }
}
