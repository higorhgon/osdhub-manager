//! Picks the device root when osdhub-manager is opened without one: a folder browser in the terminal, which then
//! offers to create the folders OSDHub uses that aren't in the folder picked (ART, CD, DVD and EMBER/games).

use crate::games::Layout;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Flex, Layout as Split};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

/// The device root picked, with the folders created in it
pub struct Picked {
    pub root: PathBuf,
    pub created: Vec<String>,
}

/// Shows the folder browser until a folder is picked, or None when it's closed
pub fn pick(terminal: &mut DefaultTerminal, layout: &Layout) -> io::Result<Option<Picked>> {
    let mut browser = Browser::new(layout, start_dir());
    loop {
        terminal.draw(|frame| browser.draw(frame))?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            browser.key(key.code);
        }
        match browser.outcome.take() {
            Some(Outcome::Picked(picked)) => return Ok(Some(picked)),
            Some(Outcome::Quit) => return Ok(None),
            None => {}
        }
    }
}

/// Where the browser starts: the folder where the desktop mounts removable drives, when there's one
fn start_dir() -> PathBuf {
    let user = env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .unwrap_or_default();
    let mut candidates = Vec::new();
    if !user.is_empty() {
        candidates.push(PathBuf::from(format!("/run/media/{user}")));
        candidates.push(PathBuf::from(format!("/media/{user}")));
    }
    candidates.push(PathBuf::from("/Volumes"));
    candidates
        .into_iter()
        .find(|dir| !cfg!(windows) && dir.is_dir())
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// The folders OSDHub uses on a device
fn device_folders(layout: &Layout) -> [String; 4] {
    [
        "ART".to_string(),
        layout.cd_folder.clone(),
        layout.dvd_folder.clone(),
        "EMBER/games".to_string(),
    ]
}

enum Outcome {
    Picked(Picked),
    Quit,
}

enum Entry {
    /// Picks the folder being browsed
    UseThis,
    Parent,
    Folder {
        name: String,
        path: PathBuf,
        /// The OSDHub folders in it, which tell a device root
        device: Vec<String>,
    },
}

/// Asks to create the OSDHub folders that a picked folder doesn't have
struct Confirm {
    root: PathBuf,
    missing: Vec<String>,
    error: Option<String>,
}

struct Browser {
    folders: [String; 4],
    /// The folder being browsed, or an empty path for the list of drives (Windows)
    dir: PathBuf,
    entries: Vec<Entry>,
    list: ListState,
    error: Option<String>,
    confirm: Option<Confirm>,
    outcome: Option<Outcome>,
}

impl Browser {
    fn new(layout: &Layout, start: PathBuf) -> Browser {
        let mut browser = Browser {
            folders: device_folders(layout),
            dir: PathBuf::new(),
            entries: Vec::new(),
            list: ListState::default(),
            error: None,
            confirm: None,
            outcome: None,
        };
        if let Err(e) = browser.open(start.clone()) {
            browser.error = Some(format!("{}: {e}", start.display()));
            let _ = browser.open(home_dir().unwrap_or_else(|| PathBuf::from("/")));
        }
        browser
    }

    /// Lists the subfolders of `dir`, with the drives for an empty path
    fn open(&mut self, dir: PathBuf) -> io::Result<()> {
        let mut entries = Vec::new();
        let mut folders: Vec<(String, PathBuf)> = if dir.as_os_str().is_empty() {
            ('A'..='Z')
                .map(|drive| (format!("{drive}:"), PathBuf::from(format!("{drive}:\\"))))
                .filter(|(_, path)| path.is_dir())
                .collect()
        } else {
            entries.push(Entry::UseThis);
            if dir.parent().is_some() || cfg!(windows) {
                entries.push(Entry::Parent);
            }
            fs::read_dir(&dir)?
                .filter_map(|e| e.ok())
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                .filter(|(name, path)| !name.starts_with('.') && path.is_dir())
                .collect()
        };
        folders.sort_by_key(|(name, _)| name.to_lowercase());
        entries.extend(folders.into_iter().map(|(name, path)| {
            let device = self
                .folders
                .iter()
                .filter(|folder| path.join(folder).is_dir())
                .cloned()
                .collect();
            Entry::Folder { name, path, device }
        }));
        self.dir = dir;
        self.entries = entries;
        self.list.select(Some(0));
        Ok(())
    }

    fn go(&mut self, dir: PathBuf) {
        self.error = None;
        if let Err(e) = self.open(dir.clone()) {
            self.error = Some(format!("{}: {e}", dir.display()));
        }
    }

    /// Goes to the parent folder, with the folder left selected
    fn go_up(&mut self) {
        if self.dir.as_os_str().is_empty() {
            return;
        }
        let left = self.dir.clone();
        let parent = match left.parent() {
            Some(parent) => parent.to_path_buf(),
            // The drives, from the root of a drive
            None if cfg!(windows) => PathBuf::new(),
            None => return,
        };
        self.go(parent);
        let index = self.entries.iter().position(|entry| match entry {
            Entry::Folder { path, .. } => {
                path == &left || (path.parent().is_none() && left.starts_with(path))
            }
            _ => false,
        });
        if index.is_some() {
            self.list.select(index);
        }
    }

    fn selected(&self) -> Option<&Entry> {
        self.list.selected().and_then(|i| self.entries.get(i))
    }

    /// Picks `root`, asking first to create the OSDHub folders it doesn't have
    fn pick(&mut self, root: PathBuf) {
        let missing: Vec<String> = self
            .folders
            .iter()
            .filter(|folder| !root.join(folder).is_dir())
            .cloned()
            .collect();
        if missing.is_empty() {
            self.outcome = Some(Outcome::Picked(Picked {
                root,
                created: Vec::new(),
            }));
        } else {
            self.confirm = Some(Confirm {
                root,
                missing,
                error: None,
            });
        }
    }

    fn key(&mut self, code: KeyCode) {
        if let Some(confirm) = &mut self.confirm {
            match code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let created = confirm
                        .missing
                        .iter()
                        .try_for_each(|folder| fs::create_dir_all(confirm.root.join(folder)));
                    match created {
                        Ok(()) => {
                            let Confirm { root, missing, .. } = self.confirm.take().unwrap();
                            self.outcome = Some(Outcome::Picked(Picked {
                                root,
                                created: missing,
                            }));
                        }
                        Err(e) => confirm.error = Some(e.to_string()),
                    }
                }
                KeyCode::Char('n') => {
                    let Confirm { root, .. } = self.confirm.take().unwrap();
                    self.outcome = Some(Outcome::Picked(Picked {
                        root,
                        created: Vec::new(),
                    }));
                }
                KeyCode::Esc => self.confirm = None,
                _ => {}
            }
            return;
        }

        let count = self.entries.len();
        let current = self.list.selected().unwrap_or(0);
        let move_to = |delta: isize| {
            Some((current as isize + delta).clamp(0, count.saturating_sub(1) as isize) as usize)
        };
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.outcome = Some(Outcome::Quit),
            KeyCode::Down | KeyCode::Char('j') => self.list.select(move_to(1)),
            KeyCode::Up | KeyCode::Char('k') => self.list.select(move_to(-1)),
            KeyCode::PageDown => self.list.select(move_to(10)),
            KeyCode::PageUp => self.list.select(move_to(-10)),
            KeyCode::Home => self.list.select(Some(0)),
            KeyCode::End => self.list.select(Some(count.saturating_sub(1))),
            KeyCode::Left | KeyCode::Backspace => self.go_up(),
            KeyCode::Char('~') => {
                if let Some(home) = home_dir() {
                    self.go(home);
                }
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => match self.selected() {
                Some(Entry::UseThis) => self.pick(self.dir.clone()),
                Some(Entry::Parent) => self.go_up(),
                Some(Entry::Folder { path, .. }) => {
                    let path = path.clone();
                    self.go(path);
                }
                None => {}
            },
            KeyCode::Char('s') | KeyCode::Char(' ') => match self.selected() {
                Some(Entry::Folder { path, .. }) => {
                    let path = path.clone();
                    self.pick(path);
                }
                _ if !self.dir.as_os_str().is_empty() => self.pick(self.dir.clone()),
                _ => {}
            },
            _ => {}
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [list, help] =
            Split::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
        let location = if self.dir.as_os_str().is_empty() {
            "Drives".to_string()
        } else {
            self.dir.display().to_string()
        };
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|entry| match entry {
                Entry::UseThis => ListItem::new(Line::from(vec![
                    Span::from("✓ Use this folder: ").green().bold(),
                    Span::from(location.clone()),
                ])),
                Entry::Parent => ListItem::new(".."),
                Entry::Folder { name, device, .. } => {
                    let mut line = vec![Span::from(format!("{name}/"))];
                    if !device.is_empty() {
                        line.push(Span::from(format!("   OSDHub: {}", device.join(" "))).green());
                    }
                    ListItem::new(Line::from(line))
                }
            })
            .collect();
        let mut block = Block::bordered().title(format!(
            " osdhub-manager — choose the device root: {location} "
        ));
        if let Some(error) = &self.error {
            block = block.title_bottom(Line::from(format!(" {error} ")).red());
        }
        frame.render_stateful_widget(
            List::new(items).block(block).highlight_style(
                Style::new()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            list,
            &mut self.list,
        );
        frame.render_widget(
            Paragraph::new(
                "↑↓ move  Enter open  ← back  s use the selected folder  ~ home  q quit",
            )
            .dark_gray(),
            help,
        );

        if let Some(confirm) = &self.confirm {
            let [area] = Split::vertical([Constraint::Length(9)])
                .flex(Flex::Center)
                .areas(frame.area());
            let [area] = Split::horizontal([Constraint::Percentage(80)])
                .flex(Flex::Center)
                .areas(area);
            let folders: Vec<String> = confirm.missing.iter().map(|f| format!("{f}/")).collect();
            let mut lines = vec![
                Line::from(format!(
                    "These folders that OSDHub uses aren't in {}:",
                    confirm.root.display()
                )),
                Line::from(format!("  {}", folders.join("  "))).bold(),
                Line::from(""),
                Line::from("Enter/y create them   n go on without them   Esc back").bold(),
            ];
            if let Some(error) = &confirm.error {
                lines.push(Line::from(""));
                lines.push(Line::from(format!("✗ {error}")).red());
            }
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .block(Block::bordered().title(" Create the OSDHub folders? ")),
                area,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render(browser: &mut Browser) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| browser.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn take_picked(browser: &mut Browser) -> Option<Picked> {
        match browser.outcome.take() {
            Some(Outcome::Picked(picked)) => Some(picked),
            _ => None,
        }
    }

    #[test]
    fn browse() {
        let base =
            std::env::temp_dir().join(format!("osdhub-manager-browse-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        for dir in ["MMCE/CD", "MMCE/DVD", "USB", ".hidden"] {
            fs::create_dir_all(base.join(dir)).unwrap();
        }
        let layout = Layout {
            cd_folder: "CD".into(),
            dvd_folder: "DVD".into(),
        };
        let mut browser = Browser::new(&layout, base.clone());
        let screen = render(&mut browser);
        println!("{screen}");
        assert!(screen.contains("✓ Use this folder"));
        assert!(screen.contains("MMCE/   OSDHub: CD DVD"));
        assert!(screen.contains("USB/"));
        assert!(!screen.contains(".hidden"));

        // Into USB and back, where USB stays selected
        browser.key(KeyCode::End);
        browser.key(KeyCode::Enter);
        assert_eq!(browser.dir, base.join("USB"));
        browser.key(KeyCode::Left);
        assert_eq!(browser.dir, base);
        assert!(matches!(browser.selected(), Some(Entry::Folder { name, .. }) if name == "USB"));

        // Picking MMCE asks to create the folders it doesn't have; Esc goes back
        browser.key(KeyCode::Up);
        browser.key(KeyCode::Char('s'));
        let screen = render(&mut browser);
        println!("{screen}");
        assert!(screen.contains("ART/  EMBER/games/"));
        browser.key(KeyCode::Esc);
        assert!(browser.confirm.is_none() && browser.outcome.is_none());
        browser.key(KeyCode::Char('s'));
        browser.key(KeyCode::Enter);
        let picked = take_picked(&mut browser).unwrap();
        assert_eq!(picked.root, base.join("MMCE"));
        assert_eq!(picked.created, ["ART", "EMBER/games"]);
        assert!(base.join("MMCE/EMBER/games").is_dir() && base.join("MMCE/ART").is_dir());

        // A device with every folder is picked right away
        browser.key(KeyCode::Char('s'));
        assert!(take_picked(&mut browser).unwrap().created.is_empty());

        // Use this folder, without creating the folders
        browser.key(KeyCode::Home);
        browser.key(KeyCode::Enter);
        browser.key(KeyCode::Char('n'));
        let picked = take_picked(&mut browser).unwrap();
        assert_eq!(picked.root, base);
        assert!(picked.created.is_empty() && !base.join("ART").exists());

        browser.key(KeyCode::Char('q'));
        assert!(matches!(browser.outcome, Some(Outcome::Quit)));
        fs::remove_dir_all(&base).unwrap();
    }
}
