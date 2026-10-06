//! The Config tab of the terminal interface: OSDMenu's settings from `OSDMENU.CNF`, inside the device's BOOT memory
//! card or in a file, with what each one does and its default. The changes are only saved with `s`, after showing
//! them and asking; the memory card (or the file) is copied first and checked after saving.

use crate::cnf::{self, Cnf, KEYS, Key, Kind};
use crate::config::{self, FoundCard, Source};
use crate::install::Target;
use crate::install_modal::{self, InstallModal, Outcome};
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout as Split, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap,
};
use std::path::{Path, PathBuf};

const SECTIONS: [&str; 5] = [
    "OSDSYS",
    "Custom menu",
    "Discs and apps",
    "Games menu",
    "PSX menu",
];

/// A row of the settings table
#[derive(Clone, Copy, PartialEq, Debug)]
enum Entry {
    Section(&'static str),
    Known(&'static Key),
    /// A setting line that isn't one of the known settings: menu entries, repeated and unknown settings
    Line(usize),
}

/// A line of text being typed
struct Input {
    /// What the text is for
    purpose: Purpose,
    text: Vec<char>,
    cursor: usize,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Purpose {
    Value(Entry),
    Export,
    Import,
    Open,
}

pub struct ConfigTab {
    root: PathBuf,
    cards: Option<Vec<FoundCard>>,
    /// Choosing where the configuration is
    picking: bool,
    pick: ListState,
    source: Option<Source>,
    original: String,
    cnf: Cnf,
    entries: Vec<Entry>,
    table: TableState,
    input: Option<Input>,
    /// The changes to save, waiting for confirmation
    confirm: Option<Vec<(Option<String>, Option<String>)>>,
    error: Option<String>,
    /// The installer, when open
    install: Option<InstallModal>,
    /// Lines for the log of the interface
    pub log: Vec<String>,
}

impl ConfigTab {
    pub fn new(root: PathBuf) -> ConfigTab {
        ConfigTab {
            root,
            cards: None,
            picking: true,
            pick: ListState::default(),
            source: None,
            original: String::new(),
            cnf: Cnf::parse(""),
            entries: Vec::new(),
            table: TableState::default(),
            input: None,
            confirm: None,
            error: None,
            install: None,
            log: Vec::new(),
        }
    }

    /// Whether there are changes not saved
    pub fn modified(&self) -> bool {
        self.source.is_some() && self.cnf.text() != self.original
    }

    /// Whether the keys are typed into the tab, so the interface's own keys don't apply
    pub fn typing(&self) -> bool {
        self.input.is_some() || self.confirm.is_some() || self.install.is_some()
    }

    /// Finds the memory cards the first time the tab is shown, opening the one with the configuration
    pub fn show(&mut self) {
        if self.cards.is_some() {
            return;
        }
        let cards = config::find_cards(&self.root);
        let with_cnf: Vec<&FoundCard> = cards.iter().filter(|c| c.cnf == Ok(true)).collect();
        let only = match with_cnf.as_slice() {
            [card] => Some(card.path.clone()),
            _ => None,
        };
        self.cards = Some(cards);
        self.pick.select(Some(0));
        if let Some(path) = only {
            self.open(Source::Card(path));
        }
    }

    fn open(&mut self, source: Source) {
        match config::load(&source) {
            Ok(text) => {
                self.log.push(format!("Opened {}", source.describe()));
                if text.is_empty() {
                    self.log
                        .push("It has no OSDMENU.CNF yet: it's created when saving".to_string());
                }
                self.original = text.clone();
                self.cnf = Cnf::parse(&text);
                self.source = Some(source);
                self.picking = false;
                self.error = None;
                self.rebuild();
                self.table.select(
                    self.entries
                        .iter()
                        .position(|e| !matches!(e, Entry::Section(_))),
                );
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// The rows: the known settings by section, then the menu entries and the other settings in the file
    fn rebuild(&mut self) {
        let mut entries = Vec::new();
        for section in SECTIONS {
            entries.push(Entry::Section(section));
            entries.extend(
                KEYS.iter()
                    .filter(|k| k.section == section)
                    .map(Entry::Known),
            );
        }
        let settings = self.cnf.settings();
        let is_menu = |name: &str| name.contains("_OSDSYS_ITEM_");
        let menu: Vec<Entry> = settings
            .iter()
            .filter(|(_, name, _)| is_menu(name))
            .map(|(i, _, _)| Entry::Line(*i))
            .collect();
        if !menu.is_empty() {
            entries.push(Entry::Section("Menu entries"));
            entries.extend(menu);
        }
        let other: Vec<Entry> = settings
            .iter()
            .filter(|(_, name, _)| !is_menu(name) && cnf::known(name).is_none())
            .map(|(i, _, _)| Entry::Line(*i))
            .collect();
        if !other.is_empty() {
            entries.push(Entry::Section("Other settings"));
            entries.extend(other);
        }
        self.entries = entries;
    }

    fn selected(&self) -> Option<Entry> {
        self.table
            .selected()
            .and_then(|i| self.entries.get(i))
            .copied()
    }

    /// Moves to the next row that isn't a section title
    fn move_by(&mut self, delta: isize) {
        let count = self.entries.len() as isize;
        let mut row = self.table.selected().unwrap_or(0) as isize;
        let step = delta.signum();
        let mut left = delta.abs();
        while left > 0 {
            let next = row + step;
            if next < 0 || next >= count {
                break;
            }
            row = next;
            if !matches!(self.entries[row as usize], Entry::Section(_)) {
                left -= 1;
            }
        }
        if matches!(self.entries.get(row as usize), Some(Entry::Section(_))) {
            return;
        }
        self.table.select(Some(row as usize));
    }

    fn start_input(&mut self, purpose: Purpose, text: &str) {
        let text: Vec<char> = text.chars().collect();
        self.input = Some(Input {
            purpose,
            cursor: text.len(),
            text,
        });
    }

    fn default_file(&self) -> String {
        std::env::current_dir()
            .unwrap_or_default()
            .join("OSDMENU.CNF")
            .display()
            .to_string()
    }

    /// Takes the messages of the installer's download
    pub fn poll(&mut self) {
        if let Some(install) = &mut self.install {
            install.poll();
        }
    }

    /// Opens the installer for the memory card open, or the only BOOT memory card, or else a folder
    fn start_install(&mut self) {
        self.show();
        let open = match &self.source {
            Some(Source::Card(path)) => Some(path.as_path()),
            _ => None,
        };
        let card = install_modal::card_for(open, self.cards.as_deref().unwrap_or_default());
        self.install = Some(InstallModal::new(self.root.clone(), card));
    }

    pub fn key(&mut self, code: KeyCode) {
        self.error = None;
        if let Some(install) = &mut self.install {
            match install.key(code) {
                Outcome::Nothing => {}
                Outcome::Closed => self.install = None,
                Outcome::Installed { log, target } => {
                    self.log.extend(log);
                    self.install = None;
                    // The memory card changed: read it again, with its OSDMENU.CNF (the example, when it had none)
                    self.cards = Some(config::find_cards(&self.root));
                    if !self.modified() {
                        let cnf = target.path().join(config::CNF_PATH);
                        match target {
                            Target::Card(card) => self.open(Source::Card(card)),
                            Target::Folder(_) if cnf.is_file() => self.open(Source::File(cnf)),
                            Target::Folder(_) => {}
                        }
                    }
                }
            }
            return;
        }
        if self.input.is_some() {
            self.input_key(code);
            return;
        }
        if let Some(changes) = &self.confirm {
            let changes = changes.clone();
            self.confirm = None;
            if matches!(code, KeyCode::Enter | KeyCode::Char('y')) {
                self.save(changes.len());
            } else {
                self.log.push("Not saved".to_string());
            }
            return;
        }
        if self.picking {
            self.pick_key(code);
            return;
        }
        match code {
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::Home => {
                self.table.select(Some(0));
                self.move_by(1);
            }
            KeyCode::End => {
                self.table
                    .select(Some(self.entries.len().saturating_sub(1)));
            }
            KeyCode::Enter => self.change(1),
            KeyCode::Right => self.change(1),
            KeyCode::Left => self.change(-1),
            KeyCode::Char('e') => {
                if let Some(entry) = self.selected() {
                    let value = self.value_of(entry).unwrap_or_default().to_string();
                    self.start_input(Purpose::Value(entry), &value);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => self.unset(),
            KeyCode::Char('s') => {
                let changes = cnf::diff(&self.original, &self.cnf.text());
                if changes.is_empty() {
                    self.log.push("No changes to save".to_string());
                } else {
                    self.confirm = Some(changes);
                }
            }
            KeyCode::Char('u') => {
                self.cnf = Cnf::parse(&self.original);
                self.rebuild();
                self.log.push("Changes undone".to_string());
            }
            KeyCode::Char('x') => {
                let file = self.default_file();
                self.start_input(Purpose::Export, &file);
            }
            KeyCode::Char('i') => {
                let file = self.default_file();
                self.start_input(Purpose::Import, &file);
            }
            KeyCode::Char('o') => self.picking = true,
            KeyCode::Char('I') => self.start_install(),
            _ => {}
        }
    }

    fn pick_key(&mut self, code: KeyCode) {
        let count = self.cards.as_ref().map_or(0, Vec::len) + 2;
        let current = self.pick.selected().unwrap_or(0);
        match code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.pick.select(Some((current + 1).min(count - 1)))
            }
            KeyCode::Up | KeyCode::Char('k') => self.pick.select(Some(current.saturating_sub(1))),
            KeyCode::Esc if self.source.is_some() => self.picking = false,
            KeyCode::Char('I') => self.start_install(),
            KeyCode::Enter => match self.cards.as_ref().and_then(|c| c.get(current)) {
                Some(card) => match &card.cnf {
                    Ok(_) => self.open(Source::Card(card.path.clone())),
                    Err(e) => self.error = Some(e.clone()),
                },
                None if current == count - 1 => self.start_install(),
                None => {
                    let start = format!("{}{}", self.root.display(), std::path::MAIN_SEPARATOR);
                    self.start_input(Purpose::Open, &start);
                }
            },
            _ => {}
        }
    }

    fn input_key(&mut self, code: KeyCode) {
        let Some(input) = &mut self.input else {
            return;
        };
        match code {
            KeyCode::Esc => self.input = None,
            KeyCode::Enter => {
                let Input { purpose, text, .. } = self.input.take().unwrap();
                let text: String = text.into_iter().collect();
                self.submit(purpose, text);
            }
            KeyCode::Char(c) => {
                input.text.insert(input.cursor, c);
                input.cursor += 1;
            }
            KeyCode::Backspace if input.cursor > 0 => {
                input.cursor -= 1;
                input.text.remove(input.cursor);
            }
            KeyCode::Delete if input.cursor < input.text.len() => {
                input.text.remove(input.cursor);
            }
            KeyCode::Left => input.cursor = input.cursor.saturating_sub(1),
            KeyCode::Right => input.cursor = (input.cursor + 1).min(input.text.len()),
            KeyCode::Home => input.cursor = 0,
            KeyCode::End => input.cursor = input.text.len(),
            _ => {}
        }
    }

    fn submit(&mut self, purpose: Purpose, text: String) {
        match purpose {
            Purpose::Value(entry) => self.set(entry, &text),
            Purpose::Export => match std::fs::write(&text, self.cnf.text()) {
                Ok(()) => self.log.push(format!(
                    "Exported to {text}: edit it, then import it with i to see the changes and save them"
                )),
                Err(e) => self.error = Some(format!("{text}: {e}")),
            },
            Purpose::Import => match std::fs::read(&text).map(String::from_utf8) {
                Ok(Ok(imported)) => {
                    self.cnf = Cnf::parse(&imported);
                    self.rebuild();
                    self.log.push(format!("Imported {text}: s shows the changes and saves them"));
                }
                Ok(Err(_)) => self.error = Some(format!("{text} isn't UTF-8 text")),
                Err(e) => self.error = Some(format!("{text}: {e}")),
            },
            Purpose::Open => {
                let path = Path::new(&text);
                if path.is_file() {
                    self.open(Source::of(path));
                } else {
                    self.error = Some(format!("{text} isn't a file"));
                }
            }
        }
    }

    /// The value of a row in the file
    fn value_of(&self, entry: Entry) -> Option<&str> {
        match entry {
            Entry::Known(key) => self.cnf.get(key.name),
            Entry::Line(index) => match cnf::parse_line(&self.cnf.lines()[index]) {
                cnf::Line::Setting { value, .. } => Some(value),
                _ => None,
            },
            Entry::Section(_) => None,
        }
    }

    fn set(&mut self, entry: Entry, value: &str) {
        match entry {
            Entry::Known(key) => {
                if let Err(expected) = cnf::check(key, value) {
                    self.error = Some(format!("{} should be {expected}", key.name));
                    return;
                }
                self.cnf.set(key.name, value);
            }
            Entry::Line(index) => {
                let name = match cnf::parse_line(&self.cnf.lines()[index]) {
                    cnf::Line::Setting { name, .. } => name.to_string(),
                    _ => return,
                };
                self.cnf.set_line(index, &name, value);
            }
            Entry::Section(_) => return,
        }
        self.rebuild();
    }

    /// Enter and the arrows: toggles a 0/1 setting, goes through the values of a setting with a few,
    /// and types the others
    fn change(&mut self, step: isize) {
        let Some(entry) = self.selected() else {
            return;
        };
        let current = self.value_of(entry).map(str::to_string);
        let values: &[&str] = match entry {
            Entry::Known(key) => match key.kind {
                Kind::Bool => &["0", "1"],
                Kind::Choice(choices) => choices,
                _ => &[],
            },
            _ => &[],
        };
        if values.is_empty() {
            if step > 0 {
                self.start_input(Purpose::Value(entry), current.as_deref().unwrap_or(""));
            }
            return;
        }
        let Entry::Known(key) = entry else {
            return;
        };
        let now = current.as_deref().unwrap_or(key.default);
        let index = values.iter().position(|v| *v == now);
        let next = match index {
            Some(i) => (i as isize + step).rem_euclid(values.len() as isize) as usize,
            None => 0,
        };
        self.set(entry, values[next]);
    }

    /// Comments out the selected setting, so OSDMenu uses its default
    fn unset(&mut self) {
        match self.selected() {
            Some(Entry::Known(key)) => self.cnf.unset(key.name),
            Some(Entry::Line(index)) => {
                let line = self.cnf.lines()[index].clone();
                if let cnf::Line::Setting { name, .. } = cnf::parse_line(&line) {
                    let name = name.to_string();
                    let value = self
                        .value_of(Entry::Line(index))
                        .unwrap_or_default()
                        .to_string();
                    self.cnf.set_line(index, &format!("# {name}"), &value);
                }
            }
            _ => return,
        }
        self.rebuild();
    }

    fn save(&mut self, changes: usize) {
        let Some(source) = &self.source else {
            return;
        };
        let text = self.cnf.text();
        match config::save(source, &self.original, &text) {
            Ok(backup) => {
                self.log.push(format!(
                    "Saved {changes} change(s) to {}. The previous version is in {}",
                    source.describe(),
                    backup.display()
                ));
                self.original = text;
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn status(&self) -> Vec<Span<'static>> {
        match &self.source {
            None => vec![Span::from("   Choose where OSDMENU.CNF is")],
            Some(source) => {
                let mut spans = vec![Span::from(format!("   {}", source.describe()))];
                if self.modified() {
                    spans.push(Span::from("   modified (s saves)").yellow());
                }
                spans
            }
        }
    }

    pub fn help(&self) -> &'static str {
        if self.install.is_some() {
            ""
        } else if self.input.is_some() {
            "Enter confirm  Esc cancel  ←→ Home End move"
        } else if self.picking {
            "↑↓ move  Enter open  I install  Esc back  1 Games"
        } else {
            "↑↓ move  Enter/←→ change  e type  d default  s save  u undo  x export  i import  o open other  I install  1 Games  q quit"
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        if self.picking {
            self.draw_picker(frame, area);
        } else {
            self.draw_settings(frame, area);
        }
        if let Some(install) = &self.install {
            install.draw(frame);
            return;
        }
        if let Some(input) = &self.input {
            let title = match input.purpose {
                Purpose::Value(Entry::Known(key)) => format!(" {} ", key.name),
                Purpose::Value(_) => " Value ".to_string(),
                Purpose::Export => " Export the configuration to ".to_string(),
                Purpose::Import => " Import the configuration from ".to_string(),
                Purpose::Open => {
                    " Open a memory card image (.mcd, .ps2, .bin) or a .cnf file ".to_string()
                }
            };
            let mut spans: Vec<Span> = input
                .text
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let span = Span::from(c.to_string());
                    if i == input.cursor {
                        span.reversed()
                    } else {
                        span
                    }
                })
                .collect();
            if input.cursor == input.text.len() {
                spans.push(Span::from(" ").reversed());
            }
            let mut lines = vec![Line::from(spans)];
            if let Purpose::Value(Entry::Known(key)) = input.purpose {
                lines.push(Line::from(key.help).dark_gray());
            }
            popup(frame, &title, lines, 4);
        }
        if let Some(changes) = &self.confirm {
            let mut lines: Vec<Line> = Vec::new();
            for (old, new) in changes {
                if let Some(old) = old {
                    lines.push(Line::from(format!("- {old}")).red());
                }
                if let Some(new) = new {
                    lines.push(Line::from(format!("+ {new}")).green());
                }
            }
            let problems = cnf::problems(&self.cnf);
            for (line, problem) in &problems {
                lines.push(Line::from(format!("⚠ line {}: {problem}", line + 1)).yellow());
            }
            lines.push(Line::from(""));
            if let Some(source) = &self.source {
                lines.push(
                    Line::from(format!(
                        "A copy of {} is made first.",
                        source.path().display()
                    ))
                    .dark_gray(),
                );
            }
            lines.push(Line::from("Enter/y save   any other key cancels").bold());
            let height = lines.len() as u16;
            popup(frame, " Save these changes? ", lines, height);
        }
    }

    fn draw_picker(&mut self, frame: &mut Frame, area: Rect) {
        let mut items: Vec<ListItem> = Vec::new();
        for card in self.cards.iter().flatten() {
            let name = card
                .path
                .strip_prefix(&self.root)
                .unwrap_or(&card.path)
                .display()
                .to_string();
            let state = match &card.cnf {
                Ok(true) => Span::from("   OSDMENU.CNF").green(),
                Ok(false) => Span::from("   no OSDMENU.CNF (created when saving)").dark_gray(),
                Err(e) => Span::from(format!("   ✗ {e}")).red(),
            };
            items.push(ListItem::new(Line::from(vec![Span::from(name), state])));
        }
        items.push(ListItem::new("Open a memory card image or a .cnf file..."));
        items.push(ListItem::new("Install OSDHub, RiptOPL, Neutrino, Ember...").cyan());
        let mut block =
            Block::bordered().title(" Where is OSDMENU.CNF? Memory cards in MemoryCards/**/BOOT/ ");
        if let Some(error) = &self.error {
            block = block.title_bottom(Line::from(format!(" {error} ")).red());
        }
        frame.render_stateful_widget(
            List::new(items).block(block).highlight_style(
                Style::new()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            area,
            &mut self.pick,
        );
    }

    fn draw_settings(&mut self, frame: &mut Frame, area: Rect) {
        let problems = cnf::problems(&self.cnf);
        let invalid = self.cnf.invalid_line();
        let rows: Vec<Row> = self
            .entries
            .iter()
            .map(|entry| match *entry {
                Entry::Section(name) => Row::new(vec![Cell::from(name.to_string()).bold().cyan()]),
                Entry::Known(key) => {
                    let value = match self.cnf.get(key.name) {
                        Some(value) if cnf::check(key, value).is_err() => {
                            Cell::from(format!("{value} ✗")).red()
                        }
                        Some(value) => Cell::from(value.to_string()),
                        None if key.default.is_empty() => Cell::from("(not set)").dark_gray(),
                        None => Cell::from(format!("({})", key.default)).dark_gray(),
                    };
                    Row::new(vec![
                        Cell::from(format!("  {}", key.name)),
                        value,
                        Cell::from(key.help).dark_gray(),
                    ])
                }
                Entry::Line(index) => {
                    let line = &self.cnf.lines()[index];
                    let (name, value) = match cnf::parse_line(line) {
                        cnf::Line::Setting { name, value } => (name.to_string(), value.to_string()),
                        _ => (line.clone(), String::new()),
                    };
                    let after_stop = invalid.is_some_and(|stop| index > stop);
                    let cell = Cell::from(format!("  {name}"));
                    Row::new(vec![
                        if after_stop { cell.dark_gray() } else { cell },
                        Cell::from(value),
                        Cell::from(if after_stop {
                            "not read by OSDMenu"
                        } else {
                            ""
                        })
                        .red(),
                    ])
                }
            })
            .collect();
        let mut block =
            Block::bordered().title(" OSDMenu settings (the defaults are in parentheses) ");
        let message = self
            .error
            .clone()
            .map(|e| Line::from(format!(" {e} ")).red())
            .or_else(|| {
                problems.first().map(|(line, problem)| {
                    Line::from(format!(" ⚠ line {}: {problem} ", line + 1)).yellow()
                })
            });
        if let Some(message) = message {
            block = block.title_bottom(message);
        }
        let table = Table::new(
            rows,
            [
                Constraint::Length(32),
                Constraint::Length(24),
                Constraint::Fill(1),
            ],
        )
        .block(block)
        .row_highlight_style(
            Style::new()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .column_spacing(1);
        frame.render_stateful_widget(table, area, &mut self.table);
    }
}

fn popup(frame: &mut Frame, title: &str, lines: Vec<Line>, height: u16) {
    let height = (height + 2).min(frame.area().height.saturating_sub(2));
    let [area] = Split::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(frame.area());
    let [area] = Split::horizontal([Constraint::Percentage(85)])
        .flex(Flex::Center)
        .areas(area);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(title.to_string())),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memcard::tests::card_image;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::fs;

    fn render(tab: &mut ConfigTab) -> String {
        let mut terminal = Terminal::new(TestBackend::new(130, 80)).unwrap();
        terminal
            .draw(|frame| tab.draw(frame, frame.area()))
            .unwrap();
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

    fn select(tab: &mut ConfigTab, name: &str) {
        let row = tab
            .entries
            .iter()
            .position(|e| matches!(e, Entry::Known(k) if k.name == name))
            .unwrap();
        tab.table.select(Some(row));
    }

    #[test]
    fn edit_and_save() {
        let root =
            std::env::temp_dir().join(format!("osdhub-manager-config-tab-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let boot = root.join("MemoryCards/BOOT");
        fs::create_dir_all(&boot).unwrap();
        let cnf =
            "OSDSYS_menu_x = 320\r\n# games_covers = 0\r\nname_OSDSYS_ITEM_1 = Launch Disc\r\n";
        fs::write(boot.join("BootCard.mcd"), card_image(cnf.as_bytes(), false)).unwrap();

        // The only BOOT card with the configuration is opened right away
        let mut tab = ConfigTab::new(root.clone());
        tab.show();
        assert!(!tab.picking);
        let screen = render(&mut tab);
        println!("{screen}");
        assert!(screen.contains("OSDSYS_menu_x"));
        assert!(screen.contains("(110)"));
        assert!(screen.contains("name_OSDSYS_ITEM_1"));
        assert!(screen.contains("Menu entries"));

        // A 0/1 setting is toggled, a choice goes through its values and a number is typed
        select(&mut tab, "games_covers");
        tab.key(KeyCode::Enter);
        assert_eq!(tab.cnf.get("games_covers"), Some("1"));
        select(&mut tab, "games_cover_type");
        tab.key(KeyCode::Right);
        assert_eq!(tab.cnf.get("games_cover_type"), Some("ico"));
        select(&mut tab, "OSDSYS_menu_x");
        tab.key(KeyCode::Enter);
        for _ in 0..3 {
            tab.key(KeyCode::Backspace);
        }
        for c in "abc".chars() {
            tab.key(KeyCode::Char(c));
        }
        tab.key(KeyCode::Enter);
        assert!(tab.error.as_ref().unwrap().contains("whole number"));
        assert_eq!(tab.cnf.get("OSDSYS_menu_x"), Some("320"));
        tab.key(KeyCode::Char('e'));
        tab.key(KeyCode::Backspace);
        tab.key(KeyCode::Backspace);
        tab.key(KeyCode::Char('9'));
        tab.key(KeyCode::Char('0'));
        tab.key(KeyCode::Enter);
        assert_eq!(tab.cnf.get("OSDSYS_menu_x"), Some("390"));
        assert!(tab.modified());

        // Saving shows the changes first; any other key cancels
        tab.key(KeyCode::Char('s'));
        let screen = render(&mut tab);
        println!("{screen}");
        assert!(screen.contains("- OSDSYS_menu_x = 320"));
        assert!(screen.contains("+ OSDSYS_menu_x = 390"));
        assert!(screen.contains("+ games_covers = 1"));
        tab.key(KeyCode::Char('n'));
        assert!(tab.modified());
        tab.key(KeyCode::Char('s'));
        tab.key(KeyCode::Enter);
        assert!(!tab.modified(), "{:?}", tab.error);
        let saved = config::load(&Source::Card(boot.join("BootCard.mcd"))).unwrap();
        assert_eq!(
            saved,
            "OSDSYS_menu_x = 390\r\ngames_covers = 1\r\nname_OSDSYS_ITEM_1 = Launch Disc\r\ngames_cover_type = ico\r\n"
        );
        assert!(tab.log.iter().any(|l| l.contains("previous version")));

        // Back to the default
        select(&mut tab, "games_cover_type");
        tab.key(KeyCode::Char('d'));
        assert_eq!(tab.cnf.get("games_cover_type"), None);
        tab.key(KeyCode::Char('u'));
        assert_eq!(tab.cnf.get("games_cover_type"), Some("ico"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn pick() {
        let root =
            std::env::temp_dir().join(format!("osdhub-manager-config-pick-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("MemoryCards/PS2/BOOT")).unwrap();
        fs::write(
            root.join("MemoryCards/PS2/BOOT/A.mcd"),
            card_image(b"a = 1\n", false),
        )
        .unwrap();
        fs::write(
            root.join("MemoryCards/PS2/BOOT/B.mcd"),
            card_image(b"b = 1\n", false),
        )
        .unwrap();
        fs::write(root.join("my.cnf"), "OSDSYS_menu_y = 50\n").unwrap();
        let mut tab = ConfigTab::new(root.clone());
        tab.show();
        // Two cards with the configuration: one is chosen
        assert!(tab.picking);
        let screen = render(&mut tab);
        assert!(screen.contains("A.mcd   OSDMENU.CNF"));
        tab.key(KeyCode::Down);
        tab.key(KeyCode::Enter);
        assert_eq!(tab.cnf.get("b"), Some("1"));
        // The last item opens the installer, for the memory card open
        tab.key(KeyCode::Char('o'));
        tab.key(KeyCode::Down);
        tab.key(KeyCode::Down);
        tab.key(KeyCode::Enter);
        assert!(tab.install.is_some() && tab.typing());
        let screen = render(&mut tab);
        assert!(screen.contains("mc0:/BOOT/BOOT.ELF in B.mcd"));
        tab.key(KeyCode::Esc);
        assert!(tab.install.is_none());
        // Or a file
        tab.key(KeyCode::Char('o'));
        tab.key(KeyCode::Up);
        tab.key(KeyCode::Enter);
        for c in "my.cnf".chars() {
            tab.key(KeyCode::Char(c));
        }
        tab.key(KeyCode::Enter);
        assert_eq!(tab.cnf.get("OSDSYS_menu_y"), Some("50"));
        assert_eq!(tab.source, Some(Source::File(root.join("my.cnf"))));
        fs::remove_dir_all(&root).unwrap();
    }
}
