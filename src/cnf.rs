//! OSDMenu's configuration file, `OSDMENU.CNF`: reads its settings the way the patcher does, and changes them
//! keeping the rest of the file (comments, order, menu entries and unknown settings) as it is.
//!
//! The patcher's getCNFString() reads `name = value` lines: a line starting with a character before `A` (`#`, `;`,
//! a digit...) is a comment, the name is made of letters, `_` and digits, and a line without `=` after the name
//! stops the reading of the rest of the file. The launcher reads the lines starting with the names it knows,
//! so the settings are always written as `name = value` at the start of their line.

/// What a setting holds
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    /// 0 or 1
    Bool,
    Int,
    /// One of these values, as written (the patcher compares them with case)
    Choice(&'static [&'static str]),
    /// Text up to this many characters (0 for no limit)
    Text(usize),
    /// `0xRR,0xGG,0xBB,0xAA`
    Color,
}

/// A setting OSDMenu knows
#[derive(PartialEq, Debug)]
pub struct Key {
    pub name: &'static str,
    pub section: &'static str,
    pub kind: Kind,
    /// The value used when it isn't set ("" for none)
    pub default: &'static str,
    pub help: &'static str,
}

const OSDSYS: &str = "OSDSYS";
const MENU: &str = "Custom menu";
const LAUNCH: &str = "Discs and apps";
const GAMES: &str = "Games menu";
const PSX: &str = "PSX menu";

const fn key(
    name: &'static str,
    section: &'static str,
    kind: Kind,
    default: &'static str,
    help: &'static str,
) -> Key {
    Key {
        name,
        section,
        kind,
        default,
        help,
    }
}

/// The settings of the patcher and the launcher, with their defaults in the code
#[rustfmt::skip]
pub const KEYS: &[Key] = &[
    key("OSDSYS_video_mode", OSDSYS, Kind::Choice(&["AUTO", "PAL", "NTSC", "480p", "1080i"]), "AUTO", "Video mode of OSDSYS"),
    key("OSDSYS_region", OSDSYS, Kind::Choice(&["AUTO", "jap", "usa", "eur"]), "AUTO", "Region of OSDSYS: video mode, button prompts and languages"),
    key("OSDSYS_boot", OSDSYS, Kind::Choice(&["opening", "clock", "browser"]), "", "Boots into the opening (save data towers), the main menu (clock) or the Browser"),
    key("OSDSYS_Skip_Disc", OSDSYS, Kind::Bool, "1", "Doesn't launch the disc in the drive automatically"),
    key("OSDSYS_custom_menu", MENU, Kind::Bool, "1", "Shows the custom menu"),
    key("OSDSYS_scroll_menu", MENU, Kind::Bool, "1", "Infinite scrolling in the custom menu"),
    key("OSDSYS_menu_x", MENU, Kind::Int, "320", "Center of the menu, horizontally (0-640)"),
    key("OSDSYS_menu_y", MENU, Kind::Int, "110", "Center of the menu, vertically"),
    key("OSDSYS_enter_x", MENU, Kind::Int, "30", "Position of the Enter button, horizontally (-1: OSDSYS's)"),
    key("OSDSYS_enter_y", MENU, Kind::Int, "-1", "Position of the Enter button, vertically (-1: OSDSYS's)"),
    key("OSDSYS_version_x", MENU, Kind::Int, "-1", "Position of the Version button, horizontally (-1: OSDSYS's)"),
    key("OSDSYS_version_y", MENU, Kind::Int, "-1", "Position of the Version button, vertically (-1: OSDSYS's)"),
    key("OSDSYS_cursor_max_velocity", MENU, Kind::Int, "1000", "Maximum speed of the cursor"),
    key("OSDSYS_cursor_acceleration", MENU, Kind::Int, "100", "Acceleration of the cursor"),
    key("OSDSYS_left_cursor", MENU, Kind::Text(19), "", "Text of the left cursor"),
    key("OSDSYS_right_cursor", MENU, Kind::Text(19), "", "Text of the right cursor"),
    key("OSDSYS_menu_top_delimiter", MENU, Kind::Text(79), "", "Text above the menu"),
    key("OSDSYS_menu_bottom_delimiter", MENU, Kind::Text(79), "", "Text below the menu"),
    key("OSDSYS_num_displayed_items", MENU, Kind::Int, "7", "Number of menu entries shown"),
    key("OSDSYS_selected_color", MENU, Kind::Color, "0x10,0x80,0xE0,0x80", "Color of the selected entry (red, green, blue, alpha)"),
    key("OSDSYS_unselected_color", MENU, Kind::Color, "0x33,0x33,0x33,0x80", "Color of the other entries (red, green, blue, alpha)"),
    key("cdrom_skip_ps2logo", LAUNCH, Kind::Bool, "0", "Runs the discs without rom0:PS2LOGO"),
    key("cdrom_disable_gameid", LAUNCH, Kind::Bool, "0", "Doesn't show the visual Game ID when launching discs"),
    key("cdrom_use_dkwdrv", LAUNCH, Kind::Bool, "0", "Launches PS1 discs with DKWDRV"),
    key("ps1drv_enable_fast", LAUNCH, Kind::Bool, "0", "Fast disc speed for PS1 discs (without DKWDRV)"),
    key("ps1drv_enable_smooth", LAUNCH, Kind::Bool, "0", "Texture smoothing for PS1 discs (without DKWDRV)"),
    key("ps1drv_use_ps1vn", LAUNCH, Kind::Bool, "0", "Runs PS1DRV with the PS1 Video Mode Negator"),
    key("app_gameid", LAUNCH, Kind::Bool, "0", "Shows the visual Game ID for the apps launched from the menu"),
    key("path_DKWDRV_ELF", LAUNCH, Kind::Text(49), "mc?:/BOOT/DKWDRV.ELF", "Path to DKWDRV, on the memory card"),
    key("games_device_mmce", GAMES, Kind::Bool, "0", "Lists the PS2 games on MMCE devices"),
    key("games_device_usb", GAMES, Kind::Bool, "0", "Lists the PS2 games on USB drives"),
    key("games_device_mx4sio", GAMES, Kind::Bool, "0", "Lists the PS2 games on MX4SIO"),
    key("games_device_udpfs", GAMES, Kind::Bool, "0", "Lists the PS2 games on a UDPFS server"),
    key("games_device_smb", GAMES, Kind::Bool, "0", "Lists the PS2 games on OPL's SMB share"),
    key("games_cd_folder", GAMES, Kind::Text(0), "CD", "Folder of the CD games"),
    key("games_dvd_folder", GAMES, Kind::Text(0), "DVD", "Folder of the DVD games"),
    key("games_launcher", GAMES, Kind::Choice(&["neutrino", "opl"]), "neutrino", "Launches the games with Neutrino or OPL"),
    key("games_neutrino_path", GAMES, Kind::Text(0), "", "Path to neutrino.elf"),
    key("games_opl_path", GAMES, Kind::Text(0), "", "Path to OPL (games_launcher = opl)"),
    key("games_smb_config", GAMES, Kind::Text(0), "", "Path to OPL's conf_network.cfg"),
    key("games_return_path", GAMES, Kind::Text(0), "", "App to run after \"Refresh list\" (OSDMenu when not set)"),
    key("games_mmce_gameid", GAMES, Kind::Bool, "1", "Switches MMCE devices to the game's own memory card"),
    key("games_live_scan", GAMES, Kind::Choice(&["0", "1", "2"]), "0", "Experimental, MMCE: refreshes the list without leaving OSDMenu (2: logs)"),
    key("games_covers", GAMES, Kind::Bool, "0", "MMCE: shows the cover of the selected game"),
    key("games_cover_type", GAMES, Kind::Choice(&["cov", "ico"]), "cov", "Cover shown: the case (cov) or the disc (ico)"),
    key("games_button_debug", GAMES, Kind::Bool, "0", "Shows debugging lines above the button prompts"),
    key("psx_device_mmce", PSX, Kind::Bool, "0", "Lists the PS1 games on MMCE devices (Ember)"),
    key("psx_device_usb", PSX, Kind::Bool, "0", "Lists the PS1 games on USB drives (Ember)"),
    key("psx_device_mx4sio", PSX, Kind::Bool, "0", "Lists the PS1 games on MX4SIO (Ember)"),
];

pub fn known(name: &str) -> Option<&'static Key> {
    KEYS.iter().find(|k| k.name == name)
}

/// What a line of the file is for the patcher
#[derive(PartialEq, Debug)]
pub enum Line<'a> {
    /// An empty line or a comment
    Comment,
    Setting {
        name: &'a str,
        value: &'a str,
    },
    /// A line the patcher can't read, which stops it from reading the rest of the file
    Invalid,
}

pub fn parse_line(line: &str) -> Line<'_> {
    let line = line.trim_start_matches(|c: char| c <= ' ');
    let Some(first) = line.bytes().next() else {
        return Line::Comment;
    };
    if first < b'A' {
        return Line::Comment;
    }
    let end = line
        .bytes()
        .position(|b| !(b >= b'A' || b.is_ascii_digit()))
        .unwrap_or(line.len());
    let (name, rest) = line.split_at(end);
    let Some(value) = rest
        .trim_start_matches(|c: char| c <= ' ')
        .strip_prefix('=')
    else {
        return Line::Invalid;
    };
    let value = value.trim_start_matches(|c: char| c <= ' ' && c != '\x07');
    Line::Setting { name, value }
}

/// Whether OSDMenu takes `value` for `key`
pub fn check(key: &Key, value: &str) -> Result<(), String> {
    match key.kind {
        Kind::Bool if !matches!(value, "0" | "1") => Err("0 or 1".to_string()),
        Kind::Int if value.parse::<i32>().is_err() => Err("a whole number".to_string()),
        Kind::Choice(choices) if !choices.contains(&value) => {
            Err(format!("one of {}", choices.join(", ")))
        }
        Kind::Text(max) if max > 0 && value.chars().count() > max => {
            Err(format!("up to {max} characters"))
        }
        Kind::Color => {
            let parts: Vec<&str> = value.split(',').map(str::trim).collect();
            let byte = |p: &&str| {
                let hex = p
                    .strip_prefix("0x")
                    .or_else(|| p.strip_prefix("0X"))
                    .unwrap_or(p);
                !hex.is_empty() && hex.len() <= 2 && u8::from_str_radix(hex, 16).is_ok()
            };
            if parts.len() == 4 && parts.iter().all(byte) {
                Ok(())
            } else {
                Err("four bytes like 0x10,0x80,0xE0,0x80".to_string())
            }
        }
        _ => Ok(()),
    }
}

/// The file being edited
#[derive(Clone, PartialEq, Debug)]
pub struct Cnf {
    lines: Vec<String>,
    crlf: bool,
    final_newline: bool,
}

impl Cnf {
    pub fn parse(text: &str) -> Cnf {
        let crlf = text.contains("\r\n");
        let final_newline = text.is_empty() || text.ends_with('\n');
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        if text.is_empty() {
            lines.clear();
        }
        Cnf {
            lines,
            crlf,
            final_newline,
        }
    }

    pub fn text(&self) -> String {
        let newline = if self.crlf { "\r\n" } else { "\n" };
        let mut text = self.lines.join(newline);
        if self.final_newline && !self.lines.is_empty() {
            text.push_str(newline);
        }
        text
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The settings the patcher reads, as (line, name, value), up to a line that stops it
    pub fn settings(&self) -> Vec<(usize, &str, &str)> {
        let mut settings = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            match parse_line(line) {
                Line::Comment => {}
                Line::Setting { name, value } => settings.push((index, name, value)),
                Line::Invalid => break,
            }
        }
        settings
    }

    /// The line where the patcher stops reading, if any
    pub fn invalid_line(&self) -> Option<usize> {
        self.lines
            .iter()
            .position(|l| parse_line(l) == Line::Invalid)
    }

    /// The value of a setting, from its last line as the patcher reads them in order
    pub fn get(&self, name: &str) -> Option<&str> {
        self.settings()
            .into_iter()
            .rev()
            .find(|(_, n, _)| *n == name)
            .map(|(_, _, v)| v)
    }

    fn line_of(&self, name: &str) -> Option<usize> {
        self.settings()
            .into_iter()
            .rev()
            .find(|(_, n, _)| *n == name)
            .map(|(i, _, _)| i)
    }

    /// Sets a setting in its line, or in place of a commented-out line of it (`# games_covers = 0`),
    /// or in a new line at the end
    pub fn set(&mut self, name: &str, value: &str) {
        let line = format!("{name} = {value}");
        if let Some(index) = self.line_of(name) {
            self.lines[index] = line;
            return;
        }
        let commented = self.lines.iter().position(|l| {
            let uncommented = l.trim_start().trim_start_matches(['#', ';']).trim_start();
            matches!(parse_line(uncommented), Line::Setting { name: n, .. } if n == name)
        });
        match commented {
            Some(index) => self.lines[index] = line,
            None => self.lines.push(line),
        }
    }

    /// Sets the line at `index` to a setting (for the lines that aren't single settings, like menu entries)
    pub fn set_line(&mut self, index: usize, name: &str, value: &str) {
        self.lines[index] = format!("{name} = {value}");
    }

    /// Comments out every line of a setting, so OSDMenu uses its default
    pub fn unset(&mut self, name: &str) {
        let lines: Vec<usize> = self
            .settings()
            .into_iter()
            .filter(|(_, n, _)| *n == name)
            .map(|(index, _, _)| index)
            .collect();
        for index in lines {
            self.lines[index] = format!("# {}", self.lines[index]);
        }
    }
}

/// What OSDMenu wouldn't take in the file, as (line, problem)
pub fn problems(cnf: &Cnf) -> Vec<(usize, String)> {
    let mut problems = Vec::new();
    for (index, name, value) in cnf.settings() {
        if let Some(key) = known(name)
            && let Err(expected) = check(key, value)
        {
            problems.push((
                index,
                format!("{name} should be {expected}, not \"{value}\""),
            ));
        }
    }
    if let Some(index) = cnf.invalid_line() {
        problems.push((
            index,
            "OSDMenu stops reading the file at this line, which isn't \"name = value\" nor a comment".to_string(),
        ));
    }
    problems
}

/// The lines changed from `old` to `new`, as (line removed, line added), for the confirmation before saving
pub fn diff(old: &str, new: &str) -> Vec<(Option<String>, Option<String>)> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // Longest common subsequence of the lines
    let mut lcs = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let mut changes = Vec::new();
    let mut flush = |removed: &mut Vec<String>, added: &mut Vec<String>| {
        let count = removed.len().max(added.len());
        for k in 0..count {
            changes.push((removed.get(k).cloned(), added.get(k).cloned()));
        }
        removed.clear();
        added.clear();
    };
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            flush(&mut removed, &mut added);
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            added.push(b[j].to_string());
            j += 1;
        } else {
            removed.push(a[i].to_string());
            i += 1;
        }
    }
    flush(&mut removed, &mut added);
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = "OSDSYS_video_mode = AUTO\r\nOSDSYS_menu_x = 320\r\nOSDSYS_left_cursor =\r\n\
        # games_covers = 0\r\n# --------\r\nname_OSDSYS_ITEM_1 = Launch Disc\r\npath1_OSDSYS_ITEM_1 = cdrom\r\n";

    #[test]
    fn lines() {
        assert_eq!(
            parse_line("  OSDSYS_menu_x  =  400 "),
            Line::Setting {
                name: "OSDSYS_menu_x",
                value: "400 "
            }
        );
        assert_eq!(
            parse_line("OSDSYS_left_cursor ="),
            Line::Setting {
                name: "OSDSYS_left_cursor",
                value: ""
            }
        );
        assert_eq!(parse_line("# games_covers = 1"), Line::Comment);
        assert_eq!(parse_line("; note"), Line::Comment);
        assert_eq!(parse_line(""), Line::Comment);
        assert_eq!(parse_line("OSDSYS menu_x = 1"), Line::Invalid);
        assert_eq!(parse_line("[section]"), Line::Invalid);
    }

    #[test]
    fn edit() {
        let mut cnf = Cnf::parse(EXAMPLE);
        assert_eq!(cnf.text(), EXAMPLE);
        assert_eq!(cnf.get("OSDSYS_menu_x"), Some("320"));
        assert_eq!(cnf.get("OSDSYS_left_cursor"), Some(""));
        assert_eq!(cnf.get("games_covers"), None);

        cnf.set("OSDSYS_menu_x", "400");
        cnf.set("games_covers", "1"); // In place of its commented-out line
        cnf.set("games_cover_type", "ico"); // At the end
        cnf.unset("OSDSYS_video_mode");
        let text = cnf.text();
        assert_eq!(
            text,
            "# OSDSYS_video_mode = AUTO\r\nOSDSYS_menu_x = 400\r\nOSDSYS_left_cursor =\r\ngames_covers = 1\r\n\
             # --------\r\nname_OSDSYS_ITEM_1 = Launch Disc\r\npath1_OSDSYS_ITEM_1 = cdrom\r\ngames_cover_type = ico\r\n"
        );
        assert_eq!(cnf.get("OSDSYS_video_mode"), None);
        let changes = diff(EXAMPLE, &text);
        assert_eq!(changes.len(), 4);
        assert!(changes.contains(&(
            Some("OSDSYS_menu_x = 320".into()),
            Some("OSDSYS_menu_x = 400".into())
        )));
        assert!(changes.contains(&(None, Some("games_cover_type = ico".into()))));
    }

    #[test]
    fn stops_reading() {
        let cnf = Cnf::parse("OSDSYS_menu_x = 1\nbroken line\nOSDSYS_menu_y = 2\n");
        assert_eq!(cnf.invalid_line(), Some(1));
        assert_eq!(cnf.get("OSDSYS_menu_y"), None);
    }

    #[test]
    fn problems_found() {
        let cnf = Cnf::parse("games_covers = yes\nOSDSYS_menu_x = 400\nbad line\n");
        let found = problems(&cnf);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, 0);
        assert!(found[1].1.contains("stops reading"));
    }

    #[test]
    fn values() {
        let k = |name| known(name).unwrap();
        assert!(check(k("games_covers"), "1").is_ok());
        assert!(check(k("games_covers"), "yes").is_err());
        assert!(check(k("OSDSYS_menu_x"), "-1").is_ok());
        assert!(check(k("OSDSYS_menu_x"), "abc").is_err());
        assert!(check(k("OSDSYS_video_mode"), "480p").is_ok());
        assert!(check(k("OSDSYS_video_mode"), "auto").is_err());
        assert!(check(k("OSDSYS_left_cursor"), &"x".repeat(20)).is_err());
        assert!(check(k("OSDSYS_selected_color"), "0x10,0x80,0xE0,0x80").is_ok());
        assert!(check(k("OSDSYS_selected_color"), "0x10,0x80,0xE0").is_err());
        assert!(check(k("OSDSYS_selected_color"), "0x10,0x80,0xE0,0x800").is_err());
    }
}
