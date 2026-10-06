//! Renames PS2 ISOs to OPL's `<title ID>.<name>.iso` form (`SLUS_202.12.BLOODY ROAR 3.iso`), which OPL needs
//! to find the game's art, configuration and cheats. OSDHub shows the same name either way.
//! The names of the ISOs and of the PS1 game folders can be edited too, to fit on OSDHub's menu; the PS1 games
//! don't need any particular name, so renaming them is optional.

use crate::games::{Console, Game};
use crate::osdhub::OPL_MAX_NAME_CHARS;
use std::fs;
use std::path::{Path, PathBuf};

/// OSDHub skips the PS1 game folders with longer names, in bytes (the launcher's GAMES_NAME_LEN)
pub const PS1_MAX_NAME_BYTES: usize = 127;

/// Characters that FAT and exFAT, the file systems of the devices, don't allow in file names
const INVALID_CHARS: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

pub enum Plan {
    /// Rename `from` to `to`
    Rename { from: PathBuf, to: PathBuf },
    /// Already starts with the title ID
    AlreadyNamed,
    /// Can't be renamed, and why
    Skip(String),
}

pub fn plan(game: &Game) -> Plan {
    if game.console != Console::Ps2 {
        return Plan::Skip("not a PS2 game".to_string());
    }
    let Some(id) = &game.id else {
        return Plan::Skip("title ID not found".to_string());
    };
    let file = file_name(&game.path);
    let (prefix, rest) = split_id_prefix(&file);
    if prefix.as_deref() == Some(file.get(..11).unwrap_or(""))
        && prefix.as_deref() == Some(id.as_str())
    {
        return Plan::AlreadyNamed;
    }
    // OPL only lists the ISOs right in the CD/DVD folders
    if game.subfolder {
        return Plan::Skip("in its own subfolder, which OPL doesn't list".to_string());
    }
    let Some(dir) = game.path.parent() else {
        return Plan::Skip("no parent folder".to_string());
    };
    let to = dir.join(format!("{id}.{rest}"));
    if to.exists() {
        return Plan::Skip(format!("{} already exists", to.display()));
    }
    Plan::Rename {
        from: game.path.clone(),
        to,
    }
}

/// Splits a title ID prefix in any form (`SLUS_202.12.`, `SLUS-202.12.` or `SLUS-20212.`) from a file name,
/// so a misnamed ISO gets the right prefix instead of a second one
fn split_id_prefix(file: &str) -> (Option<String>, &str) {
    let b = file.as_bytes();
    for len in [11, 10] {
        if b.len() > len + 1
            && b[len] == b'.'
            && file.is_char_boundary(len)
            && let Some(id) = crate::disc::parse_id(&file[..len])
        {
            return (Some(id), &file[len + 1..]);
        }
    }
    (None, file)
}

/// The parts of a game's file or folder name: the PS2 ISOs' title ID prefix (`SLUS_202.12.`) and extension, which
/// the name editor keeps, and the name between them, the one OSDHub and OPL show. A PS1 game folder is all name
pub struct Parts {
    pub console: Console,
    pub prefix: String,
    pub name: String,
    pub ext: String,
}

/// Splits the name of a PS2 ISO that OPL can list, or of a PS1 game folder, into its parts
pub fn parts(game: &Game) -> Result<Parts, String> {
    if game.console == Console::Ps1 {
        return Ok(Parts {
            console: Console::Ps1,
            prefix: String::new(),
            name: file_name(&game.path),
            ext: String::new(),
        });
    }
    let Some(id) = &game.id else {
        return Err("title ID not found".to_string());
    };
    if game.subfolder {
        return Err("in its own subfolder, which OPL doesn't list".to_string());
    }
    let file = file_name(&game.path);
    let (_, rest) = split_id_prefix(&file);
    let (name, ext) = match rest.rsplit_once('.') {
        Some((name, ext)) if !name.is_empty() => (name, format!(".{ext}")),
        _ => (rest, String::new()),
    };
    Ok(Parts {
        console: Console::Ps2,
        prefix: format!("{id}."),
        name: name.to_string(),
        ext,
    })
}

/// Whether OPL can list a PS2 ISO with this name (without the title ID and the extension),
/// or OSDHub a PS1 game folder
pub fn check_name(console: Console, name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("the name is empty".to_string());
    }
    if let Some(c) = name
        .chars()
        .find(|c| INVALID_CHARS.contains(c) || c.is_control())
    {
        return Err(format!("the name can't contain {c:?}"));
    }
    if console == Console::Ps1 {
        let bytes = name.trim().len();
        if bytes > PS1_MAX_NAME_BYTES {
            return Err(format!(
                "OSDHub skips the folders with names over {PS1_MAX_NAME_BYTES} bytes ({bytes})"
            ));
        }
        return Ok(());
    }
    let count = name.trim().chars().count();
    if count > OPL_MAX_NAME_CHARS {
        return Err(format!(
            "OPL doesn't list ISOs with names over {OPL_MAX_NAME_CHARS} characters ({count})"
        ));
    }
    Ok(())
}

/// The path of `game` renamed to `<prefix><name><extension>`, None when it already has that name
pub fn target(game: &Game, parts: &Parts, name: &str) -> Result<Option<PathBuf>, String> {
    check_name(parts.console, name)?;
    let dir = game.path.parent().ok_or("no parent folder")?;
    let to = dir.join(format!("{}{}{}", parts.prefix, name.trim(), parts.ext));
    if to == game.path {
        return Ok(None);
    }
    // Changing only the case finds the same file on FAT and exFAT
    if to.exists() && !same_file(&to, &game.path) {
        return Err(format!("{} already exists", file_name(&to)));
    }
    Ok(Some(to))
}

#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn apply(from: &Path, to: &Path) -> Result<(), String> {
    fs::rename(from, to).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes() {
        assert_eq!(
            split_id_prefix("SLUS_202.12.Game.iso"),
            (Some("SLUS_202.12".into()), "Game.iso")
        );
        assert_eq!(
            split_id_prefix("SLUS-20212.Game.iso"),
            (Some("SLUS_202.12".into()), "Game.iso")
        );
        assert_eq!(split_id_prefix("Game.iso"), (None, "Game.iso"));
        assert_eq!(split_id_prefix("Ação.iso"), (None, "Ação.iso"));
    }

    #[test]
    fn edited_names() {
        let dir =
            std::env::temp_dir().join(format!("osdhub-manager-rename-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SLUS-20212.Bloody Roar 3.ISO"), b"").unwrap();
        fs::write(dir.join("SLUS_202.12.Taken.ISO"), b"").unwrap();
        let game = Game {
            console: Console::Ps2,
            path: dir.join("SLUS-20212.Bloody Roar 3.ISO"),
            name: "SLUS-20212.Bloody Roar 3".to_string(),
            id: Some("SLUS_202.12".to_string()),
            id_error: None,
            subfolder: false,
        };
        let parts = parts(&game).unwrap();
        assert_eq!(
            (
                parts.prefix.as_str(),
                parts.name.as_str(),
                parts.ext.as_str()
            ),
            ("SLUS_202.12.", "Bloody Roar 3", ".ISO")
        );
        assert_eq!(
            target(&game, &parts, " BR3 ").unwrap(),
            Some(dir.join("SLUS_202.12.BR3.ISO"))
        );
        assert!(
            target(&game, &parts, "Taken")
                .unwrap_err()
                .contains("exists")
        );
        assert!(target(&game, &parts, "  ").is_err());
        assert!(target(&game, &parts, "A/B").unwrap_err().contains("'/'"));
        assert!(
            target(&game, &parts, &"x".repeat(161))
                .unwrap_err()
                .contains("160")
        );
        let named = Game {
            path: dir.join("SLUS_202.12.Taken.ISO"),
            ..game
        };
        assert_eq!(target(&named, &parts, "Taken").unwrap(), None);

        // A PS1 game folder is renamed whole, up to OSDHub's limit
        fs::create_dir_all(dir.join("Crash Bandicoot (USA)")).unwrap();
        let ps1 = Game {
            console: Console::Ps1,
            path: dir.join("Crash Bandicoot (USA)"),
            name: "Crash Bandicoot (USA)".to_string(),
            id: Some("SCUS_949.00".to_string()),
            id_error: None,
            subfolder: false,
        };
        let parts = super::parts(&ps1).unwrap();
        assert_eq!(parts.name, "Crash Bandicoot (USA)");
        assert_eq!(
            target(&ps1, &parts, "Crash Bandicoot").unwrap(),
            Some(dir.join("Crash Bandicoot"))
        );
        assert!(
            target(&ps1, &parts, &"ã".repeat(64))
                .unwrap_err()
                .contains("127 bytes")
        );
        assert!(target(&ps1, &parts, &"a".repeat(127)).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }
}
