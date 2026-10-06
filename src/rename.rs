//! Renames PS2 ISOs to OPL's `<title ID>.<name>.iso` form (`SLUS_202.12.BLOODY ROAR 3.iso`), which OPL needs
//! to find the game's art, configuration and cheats. OSDHub shows the same name either way.

use crate::games::{Console, Game};
use std::fs;
use std::path::PathBuf;

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
    let file = game
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
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

pub fn apply(from: &PathBuf, to: &PathBuf) -> Result<(), String> {
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
}
