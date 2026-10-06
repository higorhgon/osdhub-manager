//! The installer of the Config tab: where OSDHub goes (the BOOT memory card image, or a folder to copy to a memory
//! card), what else to install (RiptOPL, Neutrino, Ember), then the download in the background, then the files that
//! will be written, which are only written after confirming.

use crate::install::{self, Choice, Plan, Target};
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout as Split};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

const OSDHUB: usize = 0;
const INTO: usize = 1;
const FOLDER: usize = 2;
const OPL: usize = 3;
const OPL_RA: usize = 4;
const NEUTRINO: usize = 5;
const EMBER: usize = 6;
const BIOS: usize = 7;
const START: usize = 8;

enum Message {
    Progress(String),
    Done(Result<Plan, String>),
}

/// The text being typed
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Folder,
    Bios,
}

enum Stage {
    Choosing,
    Typing {
        field: Field,
        text: Vec<char>,
        cursor: usize,
    },
    Downloading {
        rx: Receiver<Message>,
        lines: Vec<String>,
    },
    Confirm {
        plan: Plan,
        lines: Vec<(String, bool)>,
    },
    Failed(String),
}

/// What happened after a key
pub enum Outcome {
    Nothing,
    Closed,
    Installed { log: Vec<String>, target: Target },
}

pub struct InstallModal {
    root: PathBuf,
    /// The BOOT memory card image, when there's one
    card: Option<PathBuf>,
    folder: PathBuf,
    /// OSDHub goes in the folder, not the memory card image
    in_folder: bool,
    choice: Choice,
    cursor: usize,
    stage: Stage,
}

impl InstallModal {
    /// The installer, into `card` when there's one, or else into a folder on the device
    pub fn new(root: PathBuf, card: Option<PathBuf>) -> InstallModal {
        InstallModal {
            folder: root.join(install::FOLDER),
            in_folder: card.is_none(),
            card,
            root,
            choice: Choice::default(),
            cursor: OPL,
            stage: Stage::Choosing,
        }
    }

    fn target(&self) -> Target {
        match &self.card {
            Some(card) if !self.in_folder => Target::Card(card.clone()),
            _ => Target::Folder(self.folder.clone()),
        }
    }

    /// Takes the messages of the download
    pub fn poll(&mut self) {
        let Stage::Downloading { rx, lines } = &mut self.stage else {
            return;
        };
        let mut done = None;
        for message in rx.try_iter() {
            match message {
                Message::Progress(line) => lines.push(line),
                Message::Done(result) => done = Some(result),
            }
        }
        match done {
            Some(Ok(plan)) => {
                let lines = plan.describe(&self.root);
                self.stage = Stage::Confirm { plan, lines };
            }
            Some(Err(e)) => self.stage = Stage::Failed(e),
            None => {}
        }
    }

    fn start(&mut self) {
        let (tx, rx) = mpsc::channel();
        let (root, target, choice) = (self.root.clone(), self.target(), self.choice.clone());
        std::thread::spawn(move || {
            let progress = |line: String| {
                let _ = tx.send(Message::Progress(line));
            };
            let result = install::prepare(&root, &target, &choice, &progress);
            let _ = tx.send(Message::Done(result));
        });
        self.stage = Stage::Downloading {
            rx,
            lines: Vec::new(),
        };
    }

    fn type_into(&mut self, field: Field) {
        let current = match field {
            Field::Folder => Some(&self.folder),
            Field::Bios => self.choice.bios.as_ref(),
        };
        let text: Vec<char> = current
            .map(|p| p.display().to_string())
            .unwrap_or_default()
            .chars()
            .collect();
        self.stage = Stage::Typing {
            field,
            cursor: text.len(),
            text,
        };
    }

    pub fn key(&mut self, code: KeyCode) -> Outcome {
        match &mut self.stage {
            Stage::Choosing => match code {
                KeyCode::Esc | KeyCode::Char('q') => return Outcome::Closed,
                KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(START),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.cursor = self.cursor.saturating_sub(1).max(INTO)
                }
                KeyCode::Enter | KeyCode::Char(' ') => match self.cursor {
                    // A memory card image only when there's one
                    INTO => self.in_folder = self.card.is_none() || !self.in_folder,
                    FOLDER => {
                        self.in_folder = true;
                        self.type_into(Field::Folder);
                    }
                    OPL => {
                        self.choice.opl = !self.choice.opl;
                        self.choice.opl_ra &= self.choice.opl;
                    }
                    OPL_RA => {
                        self.choice.opl_ra = !self.choice.opl_ra;
                        self.choice.opl |= self.choice.opl_ra;
                    }
                    NEUTRINO => self.choice.neutrino = !self.choice.neutrino,
                    EMBER => self.choice.ember = !self.choice.ember,
                    BIOS => self.type_into(Field::Bios),
                    START if code == KeyCode::Enter => self.start(),
                    _ => {}
                },
                _ => {}
            },
            Stage::Typing {
                field,
                text,
                cursor,
            } => match code {
                KeyCode::Esc => self.stage = Stage::Choosing,
                KeyCode::Enter => {
                    let value: String = text.iter().collect();
                    let value = value.trim();
                    match field {
                        Field::Folder if !value.is_empty() => self.folder = PathBuf::from(value),
                        Field::Folder => {}
                        Field::Bios => {
                            self.choice.bios = (!value.is_empty()).then(|| PathBuf::from(value));
                            self.choice.ember |= self.choice.bios.is_some();
                        }
                    }
                    self.stage = Stage::Choosing;
                }
                KeyCode::Char(c) => {
                    text.insert(*cursor, c);
                    *cursor += 1;
                }
                KeyCode::Backspace if *cursor > 0 => {
                    *cursor -= 1;
                    text.remove(*cursor);
                }
                KeyCode::Delete if *cursor < text.len() => {
                    text.remove(*cursor);
                }
                KeyCode::Left => *cursor = cursor.saturating_sub(1),
                KeyCode::Right => *cursor = (*cursor + 1).min(text.len()),
                KeyCode::Home => *cursor = 0,
                KeyCode::End => *cursor = text.len(),
                _ => {}
            },
            // The download goes on until it's done
            Stage::Downloading { .. } => {}
            Stage::Confirm { plan, .. } => {
                if matches!(code, KeyCode::Enter | KeyCode::Char('y')) {
                    let result = plan.apply(&self.root);
                    let target = plan.target.clone();
                    let mut log: Vec<String> = plan
                        .sources
                        .iter()
                        .map(|s| format!("Installed {s}"))
                        .collect();
                    return match result {
                        Ok(lines) => {
                            log.extend(lines);
                            Outcome::Installed { log, target }
                        }
                        Err(e) => {
                            self.stage = Stage::Failed(e);
                            Outcome::Nothing
                        }
                    };
                }
                self.stage = Stage::Choosing;
            }
            Stage::Failed(_) => self.stage = Stage::Choosing,
        }
        Outcome::Nothing
    }

    pub fn draw(&self, frame: &mut Frame) {
        let mut lines: Vec<Line> = Vec::new();
        let title;
        match &self.stage {
            Stage::Choosing | Stage::Typing { .. } => {
                title = " Install ";
                let check = |on: bool| if on { "[x]" } else { "[ ]" };
                let into = match (&self.card, self.in_folder) {
                    (Some(card), false) => format!(
                        "the BOOT memory card image, as mc0:/BOOT/BOOT.ELF in {}",
                        card.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                    ),
                    _ => "a folder, to copy its BOOT and SYS-CONF to a memory card (from a USB drive, with wLaunchELF)"
                        .to_string(),
                };
                let bios = match &self.choice.bios {
                    Some(path) => path.display().to_string(),
                    None => "not given (Ember needs a BIOS dumped from your console; it isn't downloaded)"
                        .to_string(),
                };
                let rows: [(String, &str); 9] = [
                    (
                        "[x] OSDHub (always)".to_string(),
                        "the latest higorhgon/osdmenu release",
                    ),
                    (format!("      into {into}"), ""),
                    (format!("      folder: {}", self.folder.display()), ""),
                    (
                        format!(
                            "{} RiptOPL               APPS/OPL/RIPTOPL.ELF",
                            check(self.choice.opl)
                        ),
                        "with the MMCE/SMB autolaunch",
                    ),
                    (
                        format!("{}   RetroAchievements build", check(self.choice.opl_ra)),
                        "",
                    ),
                    (
                        format!(
                            "{} Neutrino              APPS/neutrino/",
                            check(self.choice.neutrino)
                        ),
                        "rickgaiser/neutrino",
                    ),
                    (
                        format!("{} Ember (beta)          EMBER/", check(self.choice.ember)),
                        "by Gageformer, github.com/Gageformer/Ember",
                    ),
                    (format!("    PS1 BIOS: {bios}"), ""),
                    ("    Download and install".to_string(), ""),
                ];
                for (i, (row, note)) in rows.into_iter().enumerate() {
                    let mut spans = vec![Span::from(row)];
                    if !note.is_empty() {
                        spans.push(Span::from(format!("   {note}")).dark_gray());
                    }
                    let mut line = Line::from(spans);
                    if i == OSDHUB || (i == FOLDER && !self.in_folder) {
                        line = line.dark_gray();
                    }
                    if i == self.cursor {
                        line = line.reversed();
                    }
                    lines.push(line);
                }
                if let Stage::Typing {
                    field,
                    text,
                    cursor,
                } = &self.stage
                {
                    lines.push(Line::from(""));
                    let label = match field {
                        Field::Folder => "Folder: ",
                        Field::Bios => "BIOS file: ",
                    };
                    let mut spans = vec![Span::from(label)];
                    for (i, c) in text.iter().enumerate() {
                        let span = Span::from(c.to_string());
                        spans.push(if i == *cursor { span.reversed() } else { span });
                    }
                    if *cursor == text.len() {
                        spans.push(Span::from(" ").reversed());
                    }
                    lines.push(Line::from(spans));
                }
                lines.push(Line::from(""));
                lines.push(
                    Line::from("Space/Enter choose   Enter on \"Download and install\" downloads   Esc close")
                        .bold(),
                );
            }
            Stage::Downloading {
                lines: progress, ..
            } => {
                title = " Install: downloading ";
                lines.extend(progress.iter().map(|l| Line::from(l.clone())));
            }
            Stage::Confirm { plan, lines: files } => {
                title = " Install these files? ";
                for source in &plan.sources {
                    lines.push(Line::from(source.clone()).cyan());
                }
                lines.push(Line::from(""));
                for (file, replaces) in files {
                    let line = Line::from(format!(
                        "{file}{}",
                        if *replaces { "   (replaces it)" } else { "" }
                    ));
                    lines.push(if *replaces { line.yellow() } else { line });
                }
                if plan.creates_cnf {
                    lines.push(
                        Line::from(
                            "There's no OSDMENU.CNF there: it gets the example, opened here after",
                        )
                        .cyan(),
                    );
                }
                for note in &plan.notes {
                    lines.push(Line::from(format!("⚠ {note}")).yellow());
                }
                lines.push(Line::from(""));
                if let Target::Card(card) = &plan.target {
                    lines.push(
                        Line::from(format!("A copy of {} is made first.", card.display()))
                            .dark_gray(),
                    );
                }
                lines.push(Line::from("Enter/y install   any other key goes back").bold());
            }
            Stage::Failed(e) => {
                title = " Install failed ";
                lines.push(Line::from(e.clone()).red());
                lines.push(Line::from(""));
                lines.push(Line::from("Any key goes back").bold());
            }
        }
        let height = (lines.len() as u16 + 2).min(frame.area().height.saturating_sub(2));
        let [area] = Split::vertical([Constraint::Length(height)])
            .flex(Flex::Center)
            .areas(frame.area());
        let [area] = Split::horizontal([Constraint::Percentage(90)])
            .flex(Flex::Center)
            .areas(area);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(title)),
            area,
        );
    }
}

/// The memory card image to install OSDHub in: the one open, or the only BOOT memory card, if any
pub fn card_for(open: Option<&Path>, cards: &[crate::config::FoundCard]) -> Option<PathBuf> {
    if let Some(card) = open {
        return Some(card.to_path_buf());
    }
    let readable: Vec<_> = cards.iter().filter(|c| c.cnf.is_ok()).collect();
    match readable.as_slice() {
        [card] => Some(card.path.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices() {
        let mut modal = InstallModal::new(
            PathBuf::from("/card"),
            Some(PathBuf::from("/card/BootCard.mcd")),
        );
        assert_eq!(
            modal.target(),
            Target::Card(PathBuf::from("/card/BootCard.mcd"))
        );
        // OSDHub can't be unchecked: the cursor never goes to it
        modal.key(KeyCode::Up);
        modal.key(KeyCode::Up);
        modal.key(KeyCode::Up);
        assert_eq!(modal.cursor, INTO);
        // Into a folder instead, which can be typed
        modal.key(KeyCode::Enter);
        assert_eq!(
            modal.target(),
            Target::Folder(PathBuf::from("/card").join(install::FOLDER))
        );
        modal.key(KeyCode::Down);
        modal.key(KeyCode::Enter);
        for _ in 0..install::FOLDER.len() {
            modal.key(KeyCode::Backspace);
        }
        for c in "MC".chars() {
            modal.key(KeyCode::Char(c));
        }
        modal.key(KeyCode::Enter);
        assert_eq!(modal.target(), Target::Folder(PathBuf::from("/card/MC")));
        // The RA build checks RiptOPL, and unchecking RiptOPL unchecks the RA build
        modal.key(KeyCode::Down);
        modal.key(KeyCode::Down);
        modal.key(KeyCode::Char(' '));
        assert!(modal.choice.opl && modal.choice.opl_ra);
        modal.key(KeyCode::Up);
        modal.key(KeyCode::Char(' '));
        assert!(!modal.choice.opl && !modal.choice.opl_ra);
        // A BIOS checks Ember
        for _ in 0..4 {
            modal.key(KeyCode::Down);
        }
        assert_eq!(modal.cursor, BIOS);
        modal.key(KeyCode::Enter);
        for c in "/home/me/scph1001.bin".chars() {
            modal.key(KeyCode::Char(c));
        }
        modal.key(KeyCode::Enter);
        assert!(modal.choice.ember);
        assert_eq!(
            modal.choice.bios,
            Some(PathBuf::from("/home/me/scph1001.bin"))
        );
        assert!(matches!(modal.key(KeyCode::Esc), Outcome::Closed));

        // Without a memory card image, only a folder
        let mut modal = InstallModal::new(PathBuf::from("/usb"), None);
        modal.cursor = INTO;
        modal.key(KeyCode::Enter);
        assert_eq!(
            modal.target(),
            Target::Folder(PathBuf::from("/usb").join(install::FOLDER))
        );
    }
}
