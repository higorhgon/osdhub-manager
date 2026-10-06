//! The game search of the terminal interface, and the region of the games.
//!
//! Like fpass's filter, a game matches when it has every term of the query, in any order and without case
//! ("crash ps2" and "PS2 Crash" find the PS2 Crash games), looked for in its system, title ID, region and name.
//! A term that isn't there can still match the name fuzzily, with its letters in order ("crsh" finds "Crash"),
//! and those games are listed after the others.

use crate::games::{Console, Game};

/// The region of a game: from its title ID prefix (SLUS is USA, SLES Europe, SLPM Japan...),
/// or from a region in its name, as in Redump's names ("Crash Bandicoot (USA)")
pub fn region(game: &Game) -> Option<&'static str> {
    if let Some(id) = &game.id
        && let Some(region) = id_region(id)
    {
        return Some(region);
    }
    let name = game.name.to_lowercase();
    [
        ("(usa", "USA"),
        ("(europe", "EUR"),
        ("(pal", "EUR"),
        ("(japan", "JPN"),
        ("(korea", "KOR"),
        ("(asia", "ASIA"),
        ("(china", "CHN"),
    ]
    .into_iter()
    .find(|(tag, _)| name.contains(tag))
    .map(|(_, region)| region)
}

fn id_region(id: &str) -> Option<&'static str> {
    let prefix = id.get(..4)?.to_ascii_uppercase();
    Some(match prefix.as_str() {
        "SCAJ" | "SLAJ" | "SCAS" | "SLAS" => "ASIA",
        "SCKA" | "SLKA" | "SCKD" => "KOR",
        "SCCS" | "SLCS" => "CHN",
        "PAPX" | "PBPX" | "PCPX" | "PTPX" => "JPN",
        _ => match &prefix[2..] {
            "US" | "UD" => "USA",
            "ES" | "ED" => "EUR",
            "PS" | "PM" | "PD" | "PN" => "JPN",
            _ => return None,
        },
    })
}

/// Other words a search can use for a system or a region
fn aliases(word: &str) -> &'static str {
    match word {
        "PS1" => "psx playstation",
        "PS2" => "playstation",
        "USA" => "ntsc-u ntscu",
        "EUR" => "europe pal",
        "JPN" => "japan ntsc-j ntscj",
        "KOR" => "korea",
        "CHN" => "china",
        _ => "",
    }
}

/// A search query
pub struct Query {
    terms: Vec<String>,
}

impl Query {
    pub fn new(query: &str) -> Query {
        Query {
            terms: query.split_whitespace().map(str::to_lowercase).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// How well `game` matches, lower is better, or None when it doesn't
    pub fn score(&self, game: &Game) -> Option<usize> {
        let console = match game.console {
            Console::Ps1 => "PS1",
            Console::Ps2 => "PS2",
        };
        let mut text = vec![console, aliases(console)];
        let mut forms = Vec::new();
        if let Some(id) = &game.id {
            // SLUS_202.12 is also found as SLUS-20212 and SLUS20212
            let digits: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            forms.push(id.clone());
            forms.push(format!(
                "{}-{}",
                &digits[..4.min(digits.len())],
                &digits[4.min(digits.len())..]
            ));
            forms.push(digits);
        }
        let region = region(game);
        if let Some(region) = region {
            text.push(region);
            text.push(aliases(region));
        }
        text.extend(forms.iter().map(String::as_str));
        text.push(&game.name);
        let text = text.join(" ").to_lowercase();
        let name = game.name.to_lowercase();

        let mut score = 0;
        for term in &self.terms {
            if text.contains(term.as_str()) {
                continue;
            }
            score += fuzzy(term, &name)?;
        }
        Some(score)
    }
}

/// Matches `term` against `name` with its letters in order (at least 3 of them, so that short terms don't match
/// almost anything), scoring the letters skipped between them
fn fuzzy(term: &str, name: &str) -> Option<usize> {
    if term.chars().count() < 3 {
        return None;
    }
    let mut chars = name.chars();
    let mut skipped = 0;
    let mut started = false;
    for wanted in term.chars() {
        loop {
            let c = chars.next()?;
            if c == wanted {
                started = true;
                break;
            }
            if started {
                skipped += 1;
            }
        }
    }
    Some(1 + skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn game(console: Console, name: &str, id: Option<&str>) -> Game {
        Game {
            console,
            path: PathBuf::from(name),
            name: name.to_string(),
            id: id.map(str::to_string),
            id_error: None,
            subfolder: false,
        }
    }

    #[test]
    fn regions() {
        let region_of = |id, name| region(&game(Console::Ps2, name, id));
        assert_eq!(region_of(Some("SLUS_202.12"), "Bloody Roar 3"), Some("USA"));
        assert_eq!(region_of(Some("SCES_503.60"), "Crash"), Some("EUR"));
        assert_eq!(region_of(Some("SLPM_685.13"), "Game"), Some("JPN"));
        assert_eq!(region_of(Some("SCKA_200.01"), "Game"), Some("KOR"));
        assert_eq!(region_of(None, "Crash Bandicoot (USA)"), Some("USA"));
        assert_eq!(region_of(None, "Unknown"), None);
    }

    #[test]
    fn queries() {
        let games = [
            game(
                Console::Ps2,
                "Crash Bandicoot - The Wrath of Cortex",
                Some("SLUS_202.38"),
            ),
            game(Console::Ps2, "Crash Twinsanity", Some("SLES_525.68")),
            game(Console::Ps1, "Crash Bandicoot (USA)", Some("SCUS_949.00")),
            game(Console::Ps2, "Gran Turismo 4", Some("SCUS_973.28")),
        ];
        let found = |query: &str| -> Vec<usize> {
            let query = Query::new(query);
            let mut found: Vec<(usize, usize)> = games
                .iter()
                .enumerate()
                .filter_map(|(i, g)| query.score(g).map(|s| (s, i)))
                .collect();
            found.sort();
            found.into_iter().map(|(_, i)| i).collect()
        };
        assert_eq!(found("crash"), [0, 1, 2]);
        assert_eq!(found("Crash PS2"), [0, 1]);
        assert_eq!(found("ps2 CRASH"), [0, 1]);
        assert_eq!(found("crash psx"), [2]);
        assert_eq!(found("crash pal"), [1]);
        assert_eq!(found("usa crash ps2"), [0]);
        assert_eq!(found("scus97328"), [3]);
        assert_eq!(found("SCUS-97328"), [3]);
        assert_eq!(found("973"), [3]);
        // Fuzzy: the letters in order, after the exact matches
        assert_eq!(found("crsh twin"), [1]);
        assert_eq!(found("grtur"), [3]);
        assert!(found("xyz").is_empty());
        assert!(found("zz").is_empty());
    }
}
