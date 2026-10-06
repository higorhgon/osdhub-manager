//! Where OSDMenu's configuration is and how it's saved: `SYS-CONF/OSDMENU.CNF` inside the memory card image an MMCE
//! device boots from (`MemoryCards/.../BOOT/*.mcd`), or a `.cnf` file.
//!
//! Saving copies the file first (`<name>.bak-<UTC date and time>`), writes the new image or file next to it and
//! moves it over the old one, then reads it again: the memory card must be readable as a whole and hold the new
//! configuration, or the copy is put back.

use crate::memcard::{self, Card};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Where OSDMenu reads its configuration on the memory card
pub const CNF_PATH: &str = "SYS-CONF/OSDMENU.CNF";

#[derive(Clone, PartialEq, Debug)]
pub enum Source {
    /// A memory card image, with OSDMENU.CNF inside
    Card(PathBuf),
    /// A .cnf file
    File(PathBuf),
}

impl Source {
    /// A memory card image by its extension, or else a .cnf file
    pub fn of(path: &Path) -> Source {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if matches!(ext.as_str(), "mcd" | "ps2" | "bin" | "mc2" | "vmc") {
            Source::Card(path.to_path_buf())
        } else {
            Source::File(path.to_path_buf())
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            Source::Card(path) | Source::File(path) => path,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Source::Card(path) => format!("{} (inside, {CNF_PATH})", path.display()),
            Source::File(path) => path.display().to_string(),
        }
    }
}

/// A memory card image found on the device
pub struct FoundCard {
    pub path: PathBuf,
    /// Whether it has OSDMENU.CNF, or why it can't be read
    pub cnf: Result<bool, String>,
}

/// The memory card images in the BOOT folders under `MemoryCards`, where MMCE devices keep the card they boot from
/// (the cards of the games, in folders named after them, aren't read)
pub fn find_cards(root: &Path) -> Vec<FoundCard> {
    let Some(cards_dir) = child_ignoring_case(root, "MemoryCards") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut pending = vec![(cards_dir, 0, false)];
    while let Some((dir, depth, boot)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if path.is_dir() && depth < 3 {
                pending.push((path, depth + 1, boot || name.eq_ignore_ascii_case("BOOT")));
            } else if boot && path.is_file() && matches!(Source::of(&path), Source::Card(_)) {
                let cnf = fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|image| Card::open(&image))
                    .map(|card| card.exists(CNF_PATH));
                found.push(FoundCard { path, cnf });
            }
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

fn child_ignoring_case(dir: &Path, name: &str) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name) && e.path().is_dir())
        .map(|e| e.path())
}

/// The configuration in `source` ("" when a memory card doesn't have one yet)
pub fn load(source: &Source) -> Result<String, String> {
    let bytes = match source {
        Source::Card(path) => {
            let image = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let card = Card::open(&image)?;
            if !card.exists(CNF_PATH) {
                return Ok(String::new());
            }
            card.read(CNF_PATH)?
        }
        Source::File(path) => fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?,
    };
    String::from_utf8(bytes)
        .map_err(|_| "OSDMENU.CNF isn't UTF-8 text, so it isn't edited".to_string())
}

/// Saves `new` in place of `original`, which must still be there, returning the copy made first
pub fn save(source: &Source, original: &str, new: &str) -> Result<PathBuf, String> {
    let path = source.path();
    if load(source)? != original {
        return Err(format!(
            "{} changed since it was opened; open it again",
            path.display()
        ));
    }
    match source {
        Source::Card(path) => write_card(path, &[(CNF_PATH, new.as_bytes())]),
        Source::File(path) => replace_checked(path, new.as_bytes(), |written| {
            if written == new.as_bytes() {
                Ok(())
            } else {
                Err("the configuration read back is different".to_string())
            }
        }),
    }
}

/// Writes files into a memory card image (creating them and their directories when needed), returning the copy
/// of the image made first. Nothing is changed when they don't fit
pub fn write_card(path: &Path, files: &[(&str, &[u8])]) -> Result<PathBuf, String> {
    let image = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut card = Card::open(&image)?;
    for (file, contents) in files {
        card.write_file(file, contents)
            .map_err(|e| format!("{file}: {e}"))?;
    }
    replace_checked(path, &card.to_bytes(), |written| {
        let card = Card::open(written)?;
        card.check()?;
        for (file, contents) in files {
            if card.read(file)? != *contents {
                return Err(format!("{file} read back is different"));
            }
        }
        Ok(())
    })
}

/// Replaces a file with `contents`: copies it to `<name>.bak-<UTC date>-<time>` first, writes the new one next to
/// it and moves it over, then reads it back and `check`s it, putting the copy back when that fails
fn replace_checked(
    path: &Path,
    contents: &[u8],
    check: impl Fn(&[u8]) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let (year, month, day, hours, minutes, seconds) = memcard::date_time(0);
    let stamp = format!("{year}{month:02}{day:02}-{hours:02}{minutes:02}{seconds:02}");
    let backup = (1..)
        .map(|n| {
            let mut backup = path.as_os_str().to_owned();
            backup.push(format!(".bak-{stamp}"));
            if n > 1 {
                backup.push(format!("-{n}"));
            }
            PathBuf::from(backup)
        })
        .find(|backup| !backup.exists())
        .unwrap();
    fs::copy(path, &backup).map_err(|e| format!("copying {} first: {e}", path.display()))?;

    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".osdhub-new");
    let temporary = PathBuf::from(temporary);
    // Flushed to the card before it takes the old one's place (with the handle it was written with:
    // Windows doesn't flush a file opened only for reading)
    let written = fs::File::create(&temporary)
        .and_then(|mut file| {
            file.write_all(contents)?;
            file.sync_all()
        })
        .and_then(|_| fs::rename(&temporary, path));
    if let Err(e) = written {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "writing {}: {e} (it wasn't changed)",
            path.display()
        ));
    }

    let verified = fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|written| check(&written));
    match verified {
        Ok(()) => Ok(backup),
        Err(why) => {
            let restored = fs::copy(&backup, path)
                .map(|_| ())
                .map_err(|e| e.to_string());
            Err(match restored {
                Ok(()) => format!(
                    "{} didn't check out after saving ({why}), so it was put back",
                    path.display()
                ),
                Err(e) => format!(
                    "{} didn't check out after saving ({why}) and couldn't be put back ({e}): copy {} over it",
                    path.display(),
                    backup.display()
                ),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memcard::tests::card_image;

    #[test]
    fn find_load_save() {
        let root =
            std::env::temp_dir().join(format!("osdhub-manager-config-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let boot = root.join("MemoryCards/PS2/BOOT");
        fs::create_dir_all(&boot).unwrap();
        fs::create_dir_all(root.join("MemoryCards/PS2/SLUS-20212")).unwrap();
        fs::write(
            boot.join("BootCard.mcd"),
            card_image(b"OSDSYS_menu_x = 320\n", false),
        )
        .unwrap();
        fs::write(boot.join("Broken.mcd"), b"nothing").unwrap();
        fs::write(
            root.join("MemoryCards/PS2/SLUS-20212/SLUS-20212-1.mcd"),
            card_image(b"x", false),
        )
        .unwrap();

        let found = find_cards(&root);
        let names: Vec<String> = found
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into())
            .collect();
        assert_eq!(names, ["BootCard.mcd", "Broken.mcd"]);
        assert_eq!(found[0].cnf, Ok(true));
        assert!(found[1].cnf.is_err());

        let source = Source::of(&boot.join("BootCard.mcd"));
        let original = load(&source).unwrap();
        assert_eq!(original, "OSDSYS_menu_x = 320\n");
        let backup = save(&source, &original, "OSDSYS_menu_x = 400\n").unwrap();
        assert_eq!(load(&source).unwrap(), "OSDSYS_menu_x = 400\n");
        assert_eq!(load(&Source::Card(backup.clone())).unwrap(), original);
        assert!(!boot.join("BootCard.mcd.osdhub-new").exists());
        // Saving over a configuration that changed meanwhile is refused
        assert!(
            save(&source, &original, "x\n")
                .unwrap_err()
                .contains("changed")
        );

        let file = Source::of(&root.join("OSDMENU.CNF"));
        fs::write(file.path(), "a = 1\n").unwrap();
        save(&file, "a = 1\n", "a = 2\n").unwrap();
        assert_eq!(load(&file).unwrap(), "a = 2\n");
        fs::remove_dir_all(&root).unwrap();
    }
}
