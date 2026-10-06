//! The installer of the Config tab: what to install (OSDHub always, RiptOPL, Neutrino, Ember), then the download
//! in the background, then the files that will be written, which are only written after confirming.

use crate::install::{self, Choice, Plan};
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout as Split};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

const OSDHUB: usize = 0;
const OPL: usize = 1;
const OPL_RA: usize = 2;
const NEUTRINO: usize = 3;
const EMBER: usize = 4;
const BIOS: usize = 5;
const START: usize = 6;

enum Message {
    Progress(String),
    Done(Result<Plan, String>),
}

enum Stage {
    Choosing,
    /// Typing the path of the BIOS
    Bios {
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
    Installed { log: Vec<String>, card: PathBuf },
}

pub struct InstallModal {
    root: PathBuf,
    card: PathBuf,
    choice: Choice,
    cursor: usize,
    stage: Stage,
}

impl InstallModal {
    pub fn new(root: PathBuf, card: PathBuf) -> InstallModal {
        InstallModal {
            root,
            card,
            choice: Choice::default(),
            cursor: OPL,
            stage: Stage::Choosing,
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
        let (root, card, choice) = (self.root.clone(), self.card.clone(), self.choice.clone());
        std::thread::spawn(move || {
            let progress = |line: String| {
                let _ = tx.send(Message::Progress(line));
            };
            let result = install::prepare(&root, &card, &choice, &progress);
            let _ = tx.send(Message::Done(result));
        });
        self.stage = Stage::Downloading {
            rx,
            lines: Vec::new(),
        };
    }

    pub fn key(&mut self, code: KeyCode) -> Outcome {
        match &mut self.stage {
            Stage::Choosing => match code {
                KeyCode::Esc | KeyCode::Char('q') => return Outcome::Closed,
                KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(START),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.cursor = self.cursor.saturating_sub(1).max(OPL)
                }
                KeyCode::Enter | KeyCode::Char(' ') => match self.cursor {
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
                    BIOS => {
                        let text: Vec<char> = self
                            .choice
                            .bios
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default()
                            .chars()
                            .collect();
                        self.stage = Stage::Bios {
                            cursor: text.len(),
                            text,
                        };
                    }
                    START if code == KeyCode::Enter => self.start(),
                    _ => {}
                },
                _ => {}
            },
            Stage::Bios { text, cursor } => match code {
                KeyCode::Esc => self.stage = Stage::Choosing,
                KeyCode::Enter => {
                    let path: String = text.iter().collect();
                    let path = path.trim();
                    self.choice.bios = if path.is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(path))
                    };
                    if self.choice.bios.is_some() {
                        self.choice.ember = true;
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
                    let card = plan.card.clone();
                    let mut log: Vec<String> = plan
                        .sources
                        .iter()
                        .map(|s| format!("Installed {s}"))
                        .collect();
                    return match result {
                        Ok(lines) => {
                            log.extend(lines);
                            Outcome::Installed { log, card }
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
            Stage::Choosing | Stage::Bios { .. } => {
                title = " Install ";
                let card = self
                    .card
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let check = |on: bool| if on { "[x]" } else { "[ ]" };
                let bios = match &self.choice.bios {
                    Some(path) => path.display().to_string(),
                    None => "not given (Ember needs a BIOS dumped from your console; it isn't downloaded)".to_string(),
                };
                let rows: [(String, &str); 7] = [
                    (
                        format!("[x] OSDHub (always)    mc0:/BOOT/BOOT.ELF in {card}"),
                        "the latest higorhgon/osdmenu release",
                    ),
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
                    if i == OSDHUB {
                        line = line.dark_gray();
                    }
                    if i == self.cursor {
                        line = line.reversed();
                    }
                    lines.push(line);
                }
                if let Stage::Bios { text, cursor } = &self.stage {
                    lines.push(Line::from(""));
                    let mut spans = vec![Span::from("BIOS file: ")];
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
                lines.push(Line::from("Space/Enter choose   Enter on \"Download and install\" downloads   Esc close").bold());
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
                    lines.push(Line::from("The memory card has no OSDMENU.CNF: it gets the example, opened here after").cyan());
                }
                for note in &plan.notes {
                    lines.push(Line::from(format!("⚠ {note}")).yellow());
                }
                lines.push(Line::from(""));
                lines.push(
                    Line::from(format!("A copy of {} is made first.", plan.card.display()))
                        .dark_gray(),
                );
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

/// The memory card to install OSDHub in: the one open, or the only BOOT memory card
pub fn card_for(
    open: Option<&Path>,
    cards: &[crate::config::FoundCard],
) -> Result<PathBuf, String> {
    if let Some(card) = open {
        return Ok(card.to_path_buf());
    }
    let readable: Vec<_> = cards.iter().filter(|c| c.cnf.is_ok()).collect();
    match readable.as_slice() {
        [card] => Ok(card.path.clone()),
        [] => Err(
            "no memory card image in MemoryCards/**/BOOT/ to install OSDHub in; open one first (o)"
                .to_string(),
        ),
        _ => Err(
            "more than one BOOT memory card: open the one to install OSDHub in first".to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices() {
        let mut modal =
            InstallModal::new(PathBuf::from("/card"), PathBuf::from("/card/BootCard.mcd"));
        // OSDHub can't be unchecked: the cursor never goes to it
        modal.key(KeyCode::Up);
        assert_eq!(modal.cursor, OPL);
        // The RA build checks RiptOPL, and unchecking RiptOPL unchecks the RA build
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
    }
}
