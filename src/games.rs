//! Finds the games on a device the same way OSDHub's launcher does:
//! - PS2: `*.iso` files in the CD and DVD folders, or subfolders of them holding exactly one `*.iso`
//! - PS1: `EMBER/games/<game>/` folders holding a `*.cue` file (games run by Ember)

use crate::disc::{self, Disc};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Console {
    Ps1,
    Ps2,
}

impl Console {
    /// The folder of the console in OPL Manager's art archive
    pub fn art_folder(self) -> &'static str {
        match self {
            Console::Ps1 => "PS1",
            Console::Ps2 => "PS2",
        }
    }
}

impl fmt::Display for Console {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.art_folder())
    }
}

pub struct Game {
    pub console: Console,
    /// The ISO file (PS2) or the game folder (PS1)
    pub path: PathBuf,
    /// The name shown in the menu: the ISO name without the OPL title ID prefix and extension, or the folder name
    pub name: String,
    /// The title ID read from the disc, or from the file or folder name when the disc can't be read
    pub id: Option<String>,
    /// Why the ID couldn't be read from the disc
    pub id_error: Option<String>,
    /// A PS2 ISO in its own subfolder (which OSDHub lists, but OPL doesn't)
    pub subfolder: bool,
}

/// Where the games are on the device
pub struct Layout {
    pub cd_folder: String,
    pub dvd_folder: String,
}

pub fn scan(root: &Path, layout: &Layout, consoles: &[Console]) -> Vec<Game> {
    let mut games = Vec::new();
    if consoles.contains(&Console::Ps2) {
        for folder in [&layout.cd_folder, &layout.dvd_folder] {
            scan_ps2_folder(&root.join(folder), &mut games);
        }
    }
    if consoles.contains(&Console::Ps1) {
        scan_ember(&root.join("EMBER").join("games"), &mut games);
    }
    games.sort_by_key(|g| (g.console != Console::Ps2, g.name.to_lowercase()));
    games
}

fn is_iso(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("iso"))
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(read) => read.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
        Err(_) => return Vec::new(),
    };
    entries.sort();
    entries
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The menu name of an ISO: without the `SLUS_202.12.` prefix and the extension, like the launcher's makeDisplayName()
pub fn display_name(file: &str) -> String {
    let mut name = file;
    let b = name.as_bytes();
    if b.len() > 12
        && b[..4].iter().all(|c| c.is_ascii_alphabetic())
        && b[4] == b'_'
        && b[5..8].iter().all(|c| c.is_ascii_digit())
        && b[8] == b'.'
        && b[9..11].iter().all(|c| c.is_ascii_digit())
        && b[11] == b'.'
    {
        name = &name[12..];
    }
    if let Some(stem) = name
        .strip_suffix(".iso")
        .or_else(|| name.strip_suffix(".ISO"))
    {
        name = stem;
    }
    name.to_string()
}

fn scan_ps2_folder(dir: &Path, games: &mut Vec<Game>) {
    for path in sorted_entries(dir) {
        if path.is_dir() {
            // A subfolder with exactly one ISO, named after the subfolder
            let isos: Vec<PathBuf> = sorted_entries(&path)
                .into_iter()
                .filter(|p| p.is_file() && is_iso(p))
                .collect();
            if let [iso] = isos.as_slice() {
                games.push(ps2_game(iso.clone(), file_name(&path), true));
            }
        } else if is_iso(&path) {
            let name = display_name(&file_name(&path));
            games.push(ps2_game(path, name, false));
        }
    }
}

fn ps2_game(path: PathBuf, name: String, subfolder: bool) -> Game {
    let (id, id_error) = match Disc::iso(&path).and_then(|mut d| d.title_id()) {
        Ok(Some(id)) => (Some(id), None),
        Ok(None) => (None, Some("no SYSTEM.CNF".to_string())),
        Err(e) => (None, Some(e.to_string())),
    };
    let id = id.or_else(|| disc::find_id(&file_name(&path)));
    Game {
        console: Console::Ps2,
        path,
        name,
        id,
        id_error,
        subfolder,
    }
}

fn scan_ember(dir: &Path, games: &mut Vec<Game>) {
    for path in sorted_entries(dir) {
        if !path.is_dir() || file_name(&path).starts_with('.') {
            continue;
        }
        let Some(cue) = sorted_entries(&path)
            .into_iter()
            .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cue")))
        else {
            continue;
        };
        let read = disc::read_cue(&cue)
            .and_then(|t| Disc::raw(&t.bin, t.sector_size, t.data_offset))
            .and_then(|mut d| d.title_id());
        let (id, id_error) = match read {
            Ok(Some(id)) => (Some(id), None),
            Ok(None) => (None, Some("no SYSTEM.CNF".to_string())),
            Err(e) => (None, Some(e.to_string())),
        };
        let name = file_name(&path);
        let id = id.or_else(|| disc::find_id(&name));
        games.push(Game {
            console: Console::Ps1,
            path,
            name,
            id,
            id_error,
            subfolder: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            display_name("SLUS_202.12.BLOODY ROAR 3.iso"),
            "BLOODY ROAR 3"
        );
        assert_eq!(display_name("God of War.iso"), "God of War");
    }
}
