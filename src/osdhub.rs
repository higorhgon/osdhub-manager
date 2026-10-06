//! How much of a game name fits on OSDHub's screen.
//!
//! OSDHub shortens the names of the games submenus with "..." when they don't fit between the cover panel
//! (`games_covers = 1`) and the right edge of the 640-unit wide screen, measuring them with OSDSYS's font,
//! which is proportional. The width is estimated here per kind of character, from the widths measured on screen
//! ("Dragon Ball Z Budokai 3" is about 270 units wide), and the room is computed like the patcher does.

/// OSDHub's menu names are cut at this many characters anyway (the patcher's NAME_LEN)
pub const MAX_NAME_CHARS: usize = 79;

/// OPL doesn't list ISOs whose name (without the title ID prefix and the extension) is longer than this
pub const OPL_MAX_NAME_CHARS: usize = 160;

/// Where OSDHub draws the menu
#[derive(Clone, Copy)]
pub struct Screen {
    /// The center of the menu, OSDSYS_menu_x
    pub menu_x: i32,
    /// Whether the cover panel is shown to the left of the menu (games_covers = 1)
    pub covers: bool,
}

impl Screen {
    /// The room for a name, in OSDSYS screen units (the screen is 640 wide)
    pub fn room(&self) -> i32 {
        // The patcher's coversFitText(): from the right of the cover panel (40 + 120 + 6 + 8) to 8 units from
        // the right edge, centered on menu_x. Without covers, the menu can use the whole screen
        let left = if self.covers { 174 } else { 8 };
        2 * (self.menu_x - left).min(640 - 8 - self.menu_x).max(0)
    }

    /// Whether `name` is shown whole on OSDHub
    pub fn fits(&self, name: &str) -> bool {
        name.chars().count() <= MAX_NAME_CHARS && text_width(name) <= self.room()
    }

    /// How many characters of `name` are shown before the "..." when it doesn't fit
    pub fn visible_chars(&self, name: &str) -> usize {
        if self.fits(name) {
            return name.chars().count();
        }
        let room = self.room() - text_width("...");
        let mut width = 0;
        name.chars()
            .take(MAX_NAME_CHARS)
            .take_while(|&c| {
                width += char_width(c);
                width <= room
            })
            .count()
    }

    /// The name as OSDHub shows it, cut with "..." when it doesn't fit
    pub fn shown(&self, name: &str) -> String {
        let visible = self.visible_chars(name);
        if visible == name.chars().count() {
            return name.to_string();
        }
        let cut: String = name.chars().take(visible).collect();
        format!("{}...", cut.trim_end())
    }

    /// Why `name` isn't shown whole on OSDHub, None when it is
    pub fn warning(&self, name: &str) -> Option<String> {
        if self.fits(name) {
            return None;
        }
        let count = name.chars().count();
        let kept: String = name.chars().take(MAX_NAME_CHARS).collect();
        if text_width(&kept) <= self.room() {
            return Some(format!(
                "OSDHub keeps only the first {MAX_NAME_CHARS} of its {count} characters"
            ));
        }
        let visible = self.visible_chars(name);
        Some(if self.covers {
            format!(
                "OSDHub shows \"{}\" ({visible} of {count} characters)",
                self.shown(name)
            )
        } else {
            // Without the cover panel, OSDHub doesn't cut the names
            format!(
                "goes past the edge of OSDHub's screen after {visible} of its {count} characters"
            )
        })
    }
}

/// Estimated width of a character in OSDSYS's menu font
fn char_width(c: char) -> i32 {
    match c {
        'M' | 'W' | 'm' | 'w' => 20,
        'I' | 'i' | 'l' | 'j' | '!' | '|' | '\'' => 6,
        'f' | 't' | 'r' => 9,
        _ if c.is_uppercase() => 17,
        _ if c.is_lowercase() => 11,
        _ if c.is_ascii_digit() => 12,
        ' ' => 7,
        _ => 8,
    }
}

/// Estimated width of a text in OSDSYS's menu font, in screen units
pub fn text_width(text: &str) -> i32 {
    text.chars().map(char_width).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        let screen = Screen {
            menu_x: 400,
            covers: true,
        };
        assert_eq!(screen.room(), 452);
        // Seen on screen: this one fits, the second one went past the edge
        assert!(screen.fits("Dragon Ball Z Budokai 3"));
        assert!(!screen.fits("HARVEST MOON - SAVE THE HOMELAND"));
        assert!(screen.visible_chars("HARVEST MOON - SAVE THE HOMELAND") < 32);
        // Without covers there's more room
        assert!(
            Screen {
                menu_x: 320,
                covers: false
            }
            .fits("HARVEST MOON - SAVE THE HOMELAND")
        );
        assert!(!screen.fits(&"a".repeat(80)));
        assert_eq!(screen.warning("Dragon Ball Z Budokai 3"), None);
        let shown = screen.shown("HARVEST MOON - SAVE THE HOMELAND");
        assert!(shown.starts_with("HARVEST MOON") && shown.ends_with("..."));
        assert!(
            screen
                .warning("HARVEST MOON - SAVE THE HOMELAND")
                .unwrap()
                .contains(&shown)
        );
        let wide = Screen {
            menu_x: 320,
            covers: false,
        };
        assert!(wide.warning(&"i".repeat(90)).unwrap().contains("first 79"));
        assert!(wide.warning(&"M".repeat(40)).unwrap().contains("edge"));
    }
}
