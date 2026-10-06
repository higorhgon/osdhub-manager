//! Terminal interface: the games of the device in a table, with their title IDs, art and OPL names,
//! where the art is downloaded and the PS2 ISOs are renamed. Downloads run in a separate thread,
//! reporting to the log at the bottom, so the interface keeps responding.
//! The art of the selected game is previewed on the right, as an image in terminals that show images
//! (kitty's protocol, Sixel or iTerm2's), or with colored half blocks in the others.
//! The names that don't fit on OSDHub's menu are shown with the part it cuts in yellow, and renaming a game
//! (a PS2 ISO, or a PS1 game folder, which is optional) opens an editor for its name, which warns when the name
//! is too long for OSDHub.

use crate::browse;
use crate::config_tab::ConfigTab;
use crate::covers::{self, ArtType, Downloader, Outcome, Sources};
use crate::games::{self, Console, Game, Layout};
use crate::osdhub::Screen;
use crate::rename::{self, Plan};
use crate::search::{self, Query};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Flex, Layout as Split, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};
use ratatui_image::picker::Picker;
use ratatui_image::picker::cap_parser::QueryStdioOptions;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{FilterType, Resize, StatefulImage};
use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

/// Which games the table shows
#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Ps2,
    Ps1,
}

impl Filter {
    fn next(self) -> Filter {
        match self {
            Filter::All => Filter::Ps2,
            Filter::Ps2 => Filter::Ps1,
            Filter::Ps1 => Filter::All,
        }
    }

    fn shows(self, console: Console) -> bool {
        match self {
            Filter::All => true,
            Filter::Ps2 => console == Console::Ps2,
            Filter::Ps1 => console == Console::Ps1,
        }
    }
}

/// The tabs of the interface
#[derive(Clone, Copy, PartialEq, Debug)]
enum Tab {
    Games,
    Config,
}

/// Messages from the download thread
enum Message {
    Log(String),
    /// An image of a game was downloaded, as (game index, art type, file name)
    Art(usize, ArtType, String),
    Progress(usize, usize),
    Done,
}

/// The art types to download, cycled with `t`
const TYPE_CHOICES: [&[ArtType]; 3] = [
    &[ArtType::Cov, ArtType::Ico],
    &[ArtType::Cov],
    &[ArtType::Ico],
];

struct App {
    root: PathBuf,
    layout: Layout,
    sources: Sources,
    games: Vec<Game>,
    /// The COV and ICO images in ART/ for each game
    art: Vec<[Option<String>; 2]>,
    filter: Filter,
    table: TableState,
    marked: BTreeSet<usize>,
    types: usize,
    force: bool,
    log: Vec<String>,
    downloads: Option<Receiver<Message>>,
    progress: (usize, usize),
    /// Where OSDHub draws the menu, for the names that don't fit
    screen: Screen,
    /// The search of the games shown, typed after `/`
    search: String,
    /// Whether the keys are typing the search
    searching: bool,
    tab: Tab,
    /// OSDMenu's configuration
    config: ConfigTab,
    /// Whether quitting was asked once with changes to the configuration not saved
    quit_warned: bool,
    /// The name editor of the ISO being renamed
    editor: Option<Editor>,
    /// The games to rename after the one in the editor
    rename_queue: VecDeque<usize>,
    /// The ISOs renamed and the ones to rename, since `r` was pressed
    rename_count: (usize, usize),
    quit: bool,
    /// Asks the terminal for its image support, or draws the images with half blocks
    query_images: bool,
    /// How images are drawn, found by querying the terminal when the interface starts
    picker: Option<Picker>,
    /// The COV and ICO images of the game being previewed
    preview: Option<(usize, [Preview; 2])>,
}

/// A previewed image
enum Preview {
    Image(Box<StatefulProtocol>),
    Missing,
    Unreadable(String),
}

/// Edits the name of a PS2 ISO, between its title ID and its extension, which stay, or of a PS1 game folder
struct Editor {
    game: usize,
    parts: rename::Parts,
    name: Vec<char>,
    cursor: usize,
    /// Which of the games being renamed this is, from 1
    position: usize,
    /// Why the last rename failed
    error: Option<String>,
}

/// The preview is only shown when the terminal is at least this wide
const PREVIEW_MIN_WIDTH: u16 = 100;
const PREVIEW_WIDTH: u16 = 32;

/// Opens the interface on `root`, or on a device root picked in a folder browser first
pub fn run(
    root: Option<PathBuf>,
    layout: Layout,
    sources: Sources,
    consoles: &[Console],
    query_images: bool,
    screen: Screen,
) -> std::io::Result<()> {
    let filter = match consoles {
        [Console::Ps2] => Filter::Ps2,
        [Console::Ps1] => Filter::Ps1,
        _ => Filter::All,
    };
    ratatui::run(|terminal| {
        let (root, created) = match root {
            Some(root) => (root, Vec::new()),
            None => match browse::pick(terminal, &layout)? {
                Some(picked) => (picked.root, picked.created),
                None => return Ok(()),
            },
        };
        terminal.draw(|frame| {
            frame.render_widget(
                Paragraph::new(format!("Reading the games on {}...", root.display())),
                frame.area(),
            )
        })?;
        let mut app = App::new(root, layout, sources, filter, query_images, screen);
        if !created.is_empty() {
            let folders: Vec<String> = created.iter().map(|f| format!("{f}/")).collect();
            app.log(format!("Created {}", folders.join(" ")));
        }
        app.rescan();
        app.main_loop(terminal)
    })
}

impl App {
    fn new(
        root: PathBuf,
        layout: Layout,
        sources: Sources,
        filter: Filter,
        query_images: bool,
        screen: Screen,
    ) -> App {
        App {
            config: ConfigTab::new(root.clone()),
            root,
            layout,
            sources,
            games: Vec::new(),
            art: Vec::new(),
            filter,
            table: TableState::default(),
            marked: BTreeSet::new(),
            types: 0,
            force: false,
            log: Vec::new(),
            downloads: None,
            progress: (0, 0),
            screen,
            search: String::new(),
            searching: false,
            tab: Tab::Games,
            quit_warned: false,
            editor: None,
            rename_queue: VecDeque::new(),
            rename_count: (0, 0),
            quit: false,
            query_images,
            picker: None,
            preview: None,
        }
    }

    fn art_dir(&self) -> PathBuf {
        self.root.join("ART")
    }

    fn rescan(&mut self) {
        self.games = games::scan(&self.root, &self.layout, &[Console::Ps2, Console::Ps1]);
        self.refresh_art();
        self.preview = None;
        self.marked.clear();
        let count = self.visible().len();
        self.table.select(if count > 0 { Some(0) } else { None });
        let ids = self.games.iter().filter(|g| g.id.is_some()).count();
        self.log(format!(
            "{} games found, {ids} with a title ID",
            self.games.len()
        ));
    }

    fn refresh_art(&mut self) {
        let art_dir = self.art_dir();
        self.art = self
            .games
            .iter()
            .map(|g| match &g.id {
                Some(id) => [
                    covers::existing_art(&art_dir, id, ArtType::Cov),
                    covers::existing_art(&art_dir, id, ArtType::Ico),
                ],
                None => [None, None],
            })
            .collect();
    }

    fn log(&mut self, line: String) {
        self.log.push(line);
        if self.log.len() > 500 {
            self.log.remove(0);
        }
    }

    /// Indices of the games shown with the current filter and search, the best matches first
    fn visible(&self) -> Vec<usize> {
        let query = Query::new(&self.search);
        let mut shown: Vec<(usize, usize)> = (0..self.games.len())
            .filter(|&i| self.filter.shows(self.games[i].console))
            .filter_map(|i| query.score(&self.games[i]).map(|score| (score, i)))
            .collect();
        if !query.is_empty() {
            shown.sort_by_key(|&(score, _)| score);
        }
        shown.into_iter().map(|(_, i)| i).collect()
    }

    fn selected(&self) -> Option<usize> {
        self.table
            .selected()
            .and_then(|row| self.visible().get(row).copied())
    }

    /// The marked games shown, or every game shown when none is marked
    fn targets(&self) -> Vec<usize> {
        let visible = self.visible();
        let marked: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|i| self.marked.contains(i))
            .collect();
        if marked.is_empty() { visible } else { marked }
    }

    fn main_loop(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        // Asks the terminal which image protocol it supports and its font size, once it's in raw mode.
        // Terminals answer right away; one that doesn't gets half blocks after the timeout
        let picker = if self.query_images {
            let options = QueryStdioOptions {
                timeout: Duration::from_secs(1),
                ..Default::default()
            };
            Picker::from_query_stdio_with_options(options).unwrap_or_else(|_| Picker::halfblocks())
        } else {
            Picker::halfblocks()
        };
        self.picker = Some(picker);
        while !self.quit {
            self.receive();
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.key(key.code);
            }
        }
        Ok(())
    }

    fn receive(&mut self) {
        let Some(rx) = &self.downloads else { return };
        let messages: Vec<Message> = rx.try_iter().collect();
        for message in messages {
            match message {
                Message::Log(line) => self.log(line),
                Message::Art(game, art, file) => {
                    self.art[game][art as usize] = Some(file);
                    if self
                        .preview
                        .as_ref()
                        .is_some_and(|(previewed, _)| *previewed == game)
                    {
                        self.preview = None;
                    }
                }
                Message::Progress(done, total) => self.progress = (done, total),
                Message::Done => {
                    self.downloads = None;
                    self.log(
                        "Done. Refresh the game lists in OSDHub to convert the new images."
                            .to_string(),
                    );
                }
            }
        }
    }

    /// Quits, unless the configuration has changes not saved, which is told once first
    fn quit(&mut self) {
        if self.config.modified() && !self.quit_warned {
            self.quit_warned = true;
            self.log(
                "OSDMENU.CNF has changes not saved: s in the Config tab saves them, q again quits"
                    .to_string(),
            );
        } else {
            self.quit = true;
        }
    }

    fn show_config(&mut self) {
        self.tab = Tab::Config;
        self.config.show();
        self.take_config_log();
    }

    fn take_config_log(&mut self) {
        for line in std::mem::take(&mut self.config.log) {
            self.log(line);
        }
    }

    fn key(&mut self, code: KeyCode) {
        if self.tab == Tab::Config {
            if !self.config.typing() {
                match code {
                    KeyCode::Char('1') => {
                        self.tab = Tab::Games;
                        return;
                    }
                    KeyCode::Char('q') => {
                        self.quit();
                        return;
                    }
                    _ => {}
                }
            }
            self.config.key(code);
            self.take_config_log();
            return;
        }
        if self.editor.is_some() {
            self.edit(code);
            return;
        }
        if self.searching {
            self.search_key(code);
            return;
        }
        let rows = self.visible().len();
        match code {
            KeyCode::Char('/') => self.searching = true,
            // Esc clears the search first
            KeyCode::Esc if !self.search.is_empty() => self.set_search(String::new()),
            KeyCode::Char('q') | KeyCode::Esc => self.quit(),
            KeyCode::Char('2') => self.show_config(),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1, rows),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1, rows),
            KeyCode::PageDown => self.move_by(10, rows),
            KeyCode::PageUp => self.move_by(-10, rows),
            KeyCode::Home => self.table.select(if rows > 0 { Some(0) } else { None }),
            KeyCode::End => self.table.select(rows.checked_sub(1)),
            KeyCode::Tab => {
                self.filter = self.filter.next();
                self.table.select(if self.visible().is_empty() {
                    None
                } else {
                    Some(0)
                });
            }
            KeyCode::Char(' ') => {
                if let Some(game) = self.selected() {
                    if !self.marked.remove(&game) {
                        self.marked.insert(game);
                    }
                    self.move_by(1, rows);
                }
            }
            KeyCode::Char('a') => {
                let visible = self.visible();
                if visible.iter().all(|i| self.marked.contains(i)) {
                    self.marked.clear();
                } else {
                    self.marked.extend(visible);
                }
            }
            KeyCode::Char('t') => self.types = (self.types + 1) % TYPE_CHOICES.len(),
            KeyCode::Char('f') => self.force = !self.force,
            KeyCode::Char('c') => self.start_downloads(),
            KeyCode::Char('r') => self.start_renames(),
            KeyCode::Char('s') if self.downloads.is_none() => self.rescan(),
            _ => {}
        }
    }

    /// Keys while typing the search: Enter keeps it, Esc clears it, and the arrows move in the games found
    fn search_key(&mut self, code: KeyCode) {
        let rows = self.visible().len();
        match code {
            KeyCode::Enter => self.searching = false,
            KeyCode::Esc => {
                self.searching = false;
                self.set_search(String::new());
            }
            KeyCode::Backspace => {
                let mut search = self.search.clone();
                search.pop();
                self.set_search(search);
            }
            KeyCode::Char(c) => {
                let search = format!("{}{c}", self.search);
                self.set_search(search);
            }
            KeyCode::Down => self.move_by(1, rows),
            KeyCode::Up => self.move_by(-1, rows),
            KeyCode::PageDown => self.move_by(10, rows),
            KeyCode::PageUp => self.move_by(-10, rows),
            _ => {}
        }
    }

    fn set_search(&mut self, search: String) {
        self.search = search;
        let rows = self.visible().len();
        self.table.select(if rows > 0 { Some(0) } else { None });
    }

    fn move_by(&mut self, delta: i32, rows: usize) {
        if rows == 0 {
            return;
        }
        let current = self.table.selected().unwrap_or(0) as i32;
        self.table
            .select(Some((current + delta).clamp(0, rows as i32 - 1) as usize));
    }

    fn start_downloads(&mut self) {
        if self.downloads.is_some() {
            self.log("A download is already running".to_string());
            return;
        }
        let jobs: Vec<(usize, Game)> = self
            .targets()
            .into_iter()
            .filter(|&i| self.games[i].id.is_some())
            .map(|i| (i, self.games[i].clone()))
            .collect();
        if jobs.is_empty() {
            self.log("No games with a title ID to download art for".to_string());
            return;
        }
        let types: Vec<ArtType> = TYPE_CHOICES[self.types].to_vec();
        let (force, art_dir, sources) = (self.force, self.art_dir(), self.sources.clone());
        let (tx, rx) = mpsc::channel();
        self.downloads = Some(rx);
        self.progress = (0, jobs.len() * types.len());
        self.log(format!("Downloading art for {} game(s)...", jobs.len()));

        std::thread::spawn(move || {
            let downloader = Downloader::new(sources);
            let total = jobs.len() * types.len();
            let mut done = 0;
            for (index, game) in &jobs {
                let id = game.id.as_deref().unwrap_or_default();
                for &art in &types {
                    let line = match downloader.download(game, id, art, &art_dir, force, false) {
                        Outcome::Exists(_) => None,
                        Outcome::Downloaded { file, source } => {
                            let _ = tx.send(Message::Art(*index, art, file.clone()));
                            Some(format!("{id} {}: {file} ({source})", game.name))
                        }
                        Outcome::NotFound => Some(format!(
                            "{id} {}: no {} image found",
                            game.name,
                            art.suffix()
                        )),
                        Outcome::Failed(e) => {
                            Some(format!("{id} {}: {} failed: {e}", game.name, art.suffix()))
                        }
                        Outcome::WouldDownload => None,
                    };
                    if let Some(line) = line {
                        let _ = tx.send(Message::Log(line));
                    }
                    done += 1;
                    let _ = tx.send(Message::Progress(done, total));
                }
            }
            let _ = tx.send(Message::Done);
        });
    }

    /// Opens the name editor for the marked games, one after the other, or for the selected one
    fn start_renames(&mut self) {
        let visible = self.visible();
        let marked: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|i| self.marked.contains(i))
            .collect();
        let games = if marked.is_empty() {
            self.selected().into_iter().collect()
        } else {
            marked
        };
        self.rename_queue = games.into_iter().collect();
        if self.rename_queue.is_empty() {
            self.log("No games to rename".to_string());
            return;
        }
        self.rename_count = (0, self.rename_queue.len());
        self.next_rename(0);
    }

    /// Opens the editor for the next game to rename, after `done` games, or ends the renames
    fn next_rename(&mut self, done: usize) {
        self.editor = None;
        let mut position = done;
        while let Some(game) = self.rename_queue.pop_front() {
            position += 1;
            match rename::parts(&self.games[game]) {
                Ok(parts) => {
                    let name: Vec<char> = parts.name.chars().collect();
                    self.editor = Some(Editor {
                        game,
                        cursor: name.len(),
                        name,
                        parts,
                        position,
                        error: None,
                    });
                    return;
                }
                Err(reason) => {
                    let line = format!("{}: not renamed, {reason}", self.games[game].name);
                    self.log(line);
                }
            }
        }
        let renamed = self.rename_count.0;
        if renamed > 0 {
            self.log(format!(
                "{renamed} game(s) renamed. Refresh the game lists in OSDHub, since the paths changed."
            ));
        }
    }

    fn edit(&mut self, code: KeyCode) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        editor.error = None;
        match code {
            KeyCode::Enter => self.rename_edited(),
            KeyCode::Tab => {
                let (position, name) = (editor.position, self.games[editor.game].name.clone());
                self.log(format!("{name}: skipped"));
                self.next_rename(position);
            }
            KeyCode::Esc => {
                self.rename_queue.clear();
                self.log("Rename cancelled".to_string());
                self.next_rename(0);
            }
            KeyCode::Char(c) => {
                editor.name.insert(editor.cursor, c);
                editor.cursor += 1;
            }
            KeyCode::Backspace if editor.cursor > 0 => {
                editor.cursor -= 1;
                editor.name.remove(editor.cursor);
            }
            KeyCode::Delete if editor.cursor < editor.name.len() => {
                editor.name.remove(editor.cursor);
            }
            KeyCode::Left => editor.cursor = editor.cursor.saturating_sub(1),
            KeyCode::Right => editor.cursor = (editor.cursor + 1).min(editor.name.len()),
            KeyCode::Home => editor.cursor = 0,
            KeyCode::End => editor.cursor = editor.name.len(),
            _ => {}
        }
    }

    /// Renames the ISO in the editor to the edited name
    fn rename_edited(&mut self) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let game = &mut self.games[editor.game];
        let name: String = editor.name.iter().collect();
        let line = match rename::target(game, &editor.parts, &name) {
            Err(e) => {
                editor.error = Some(e);
                return;
            }
            Ok(None) => format!("{}: unchanged", rename::file_name(&game.path)),
            Ok(Some(to)) => {
                if let Err(e) = rename::apply(&game.path, &to) {
                    editor.error = Some(e);
                    return;
                }
                let line = format!(
                    "{} → {}",
                    rename::file_name(&game.path),
                    rename::file_name(&to)
                );
                game.name = match game.console {
                    Console::Ps2 => games::display_name(&rename::file_name(&to)),
                    Console::Ps1 => rename::file_name(&to),
                };
                game.path = to;
                self.rename_count.0 += 1;
                line
            }
        };
        let position = editor.position;
        self.log(line);
        self.next_rename(position);
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [header, table, log, help] = Split::vertical([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(8),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        self.draw_header(frame, header);
        if self.tab == Tab::Config {
            self.config.draw(frame, table);
            self.draw_log(frame, log);
            frame.render_widget(Paragraph::new(self.config.help()).dark_gray(), help);
            return;
        }
        if table.width >= PREVIEW_MIN_WIDTH && self.picker.is_some() {
            let [table, preview] =
                Split::horizontal([Constraint::Fill(1), Constraint::Length(PREVIEW_WIDTH)])
                    .areas(table);
            self.draw_table(frame, table);
            self.draw_preview(frame, preview);
        } else {
            self.draw_table(frame, table);
        }
        self.draw_log(frame, log);
        frame.render_widget(
            Paragraph::new(
                "↑↓ move  / search  Space select  a select all  Tab PS2/PS1  c download images  t covers/discs  f keep/replace  r rename  s rescan  2 Config  q quit",
            )
            .dark_gray(),
            help,
        );
        if let Some(editor) = &self.editor {
            self.draw_editor(frame, editor);
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let tab = |label: &'static str, filter: Filter| {
            if self.filter == filter {
                Span::from(format!(" {label} ")).reversed()
            } else {
                Span::from(format!(" {label} "))
            }
        };
        let types = match TYPE_CHOICES[self.types] {
            [ArtType::Cov] => "covers only",
            [ArtType::Ico] => "discs only",
            _ => "covers + discs",
        };
        let existing = if self.force {
            Span::from("replace").yellow()
        } else {
            Span::from("keep")
        };
        let page = |label: &'static str, tab: Tab| {
            if self.tab == tab {
                Span::from(format!(" {label} ")).bold().reversed()
            } else {
                Span::from(format!(" {label} ")).dark_gray()
            }
        };
        let mut status = vec![
            page("1 Games", Tab::Games),
            page("2 Config", Tab::Config),
            Span::from("  "),
        ];
        if self.tab == Tab::Config {
            status.extend(self.config.status());
            let block =
                Block::bordered().title(format!(" osdhub-manager — {} ", self.root.display()));
            frame.render_widget(Paragraph::new(Line::from(status)).block(block), area);
            return;
        }
        status.extend([
            tab("All", Filter::All),
            tab("PS2", Filter::Ps2),
            tab("PS1", Filter::Ps1),
            Span::from(format!("   Download (t): {types}")),
            Span::from("   Images already in ART (f): "),
            existing,
            Span::from(format!("   Selected: {}", self.marked.len())),
        ]);
        if self.downloads.is_some() {
            status.push(
                Span::from(format!(
                    "   Downloading {}/{}",
                    self.progress.0, self.progress.1
                ))
                .yellow(),
            );
        }
        let block = Block::bordered().title(format!(" osdhub-manager — {} ", self.root.display()));
        frame.render_widget(Paragraph::new(Line::from(status)).block(block), area);
    }

    /// Loads the images of the selected game when the selection changes
    fn load_preview(&mut self) {
        let Some(game) = self.selected() else {
            self.preview = None;
            return;
        };
        if self
            .preview
            .as_ref()
            .is_some_and(|(previewed, _)| *previewed == game)
        {
            return;
        }
        let Some(picker) = &self.picker else { return };
        let art_dir = self.art_dir();
        let load = |file: &Option<String>| match file {
            None => Preview::Missing,
            Some(file) => match image::open(art_dir.join(file)) {
                Ok(image) => Preview::Image(Box::new(picker.new_resize_protocol(image))),
                Err(e) => Preview::Unreadable(e.to_string()),
            },
        };
        let images = [load(&self.art[game][0]), load(&self.art[game][1])];
        self.preview = Some((game, images));
    }

    fn draw_preview(&mut self, frame: &mut Frame, area: Rect) {
        self.load_preview();
        let title = match self.selected() {
            Some(game) => format!(" {} ", self.games[game].id.as_deref().unwrap_or("no ID")),
            None => " Art ".to_string(),
        };
        let block = Block::bordered().title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let Some((_, images)) = &mut self.preview else {
            return;
        };

        // The case cover above the disc, with their proportions (a cell is about twice as tall as it is wide)
        let [cov, ico] =
            Split::vertical([Constraint::Percentage(65), Constraint::Percentage(35)]).areas(inner);
        for ((image, area), label) in images.iter_mut().zip([cov, ico]).zip(["COV", "ICO"]) {
            match image {
                Preview::Image(protocol) => {
                    // Centered, keeping its proportions: the cover is scaled to the area, while the disc,
                    // a small image that breaks up when enlarged much, is shown at twice its size at most
                    let resize = Resize::Scale(Some(FilterType::Triangle));
                    let mut room = area.as_size();
                    if label == "ICO" {
                        let natural =
                            protocol.size_for(Resize::Fit(Some(FilterType::Triangle)), room);
                        room.width = room.width.min(natural.width * 2);
                        room.height = room.height.min(natural.height * 2);
                    }
                    let size = protocol.size_for(resize.clone(), room);
                    let centered = Rect {
                        x: area.x + area.width.saturating_sub(size.width) / 2,
                        y: area.y + area.height.saturating_sub(size.height) / 2,
                        width: size.width.min(area.width),
                        height: size.height.min(area.height),
                    };
                    frame.render_stateful_widget(
                        StatefulImage::default().resize(resize),
                        centered,
                        protocol.as_mut(),
                    );
                }
                Preview::Missing => frame.render_widget(
                    Paragraph::new(format!("no {label}")).dark_gray().centered(),
                    area,
                ),
                Preview::Unreadable(e) => frame.render_widget(
                    Paragraph::new(format!("{label}: {e}"))
                        .red()
                        .wrap(Wrap { trim: true }),
                    area,
                ),
            }
        }
    }

    fn draw_table(&mut self, frame: &mut Frame, area: Rect) {
        let art_cell = |file: &Option<String>| match file {
            Some(_) => Cell::from("✓").green(),
            None => Cell::from("✗").red(),
        };
        let rows: Vec<Row> = self
            .visible()
            .into_iter()
            .map(|i| {
                let game = &self.games[i];
                // Whether the game needs a new name: OPL's name for the PS2 ISOs, and ⚠ for the names
                // OSDHub cuts, which is optional to fix
                let cut = !self.screen.fits(&game.name);
                let warn = |text: &str| {
                    if cut {
                        format!("{text} ⚠").trim_start().to_string()
                    } else {
                        text.to_string()
                    }
                };
                let opl = match (game.console, rename::plan(game)) {
                    (Console::Ps1, _) | (_, Plan::AlreadyNamed) if cut => Cell::from("⚠").yellow(),
                    (Console::Ps1, _) | (_, Plan::AlreadyNamed) => Cell::from("✓").green(),
                    (_, Plan::Rename { .. }) => Cell::from(warn("rename")).yellow(),
                    (_, Plan::Skip(_)) if game.subfolder => Cell::from(warn("folder")).dark_gray(),
                    (_, Plan::Skip(_)) => Cell::from("✗").red(),
                };
                let id = match &game.id {
                    Some(id) => Cell::from(id.clone()),
                    None => Cell::from("no ID").red(),
                };
                Row::new(vec![
                    Cell::from(if self.marked.contains(&i) { "●" } else { " " }).cyan(),
                    Cell::from(game.console.to_string()),
                    id,
                    Cell::from(search::region(game).unwrap_or("-")),
                    art_cell(&self.art[i][0]),
                    art_cell(&self.art[i][1]),
                    opl,
                    Cell::from(self.name_line(&game.name)),
                ])
            })
            .collect();
        let widths = [
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(11),
            Constraint::Length(6),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(8),
            Constraint::Fill(1),
        ];
        let header =
            Row::new(["", "", "Title ID", "Region", "COV", "ICO", "Rename", "Name"]).bold();
        let mut block = Block::bordered().title(format!(" Games ({}) ", self.visible().len()));
        if self.searching || !self.search.is_empty() {
            let cursor = if self.searching { "█" } else { "" };
            let hint = if self.searching {
                "  Enter keep  Esc clear "
            } else {
                "  / edit  Esc clear "
            };
            block = block.title_bottom(Line::from(vec![
                Span::from(format!(" / {}{cursor}", self.search))
                    .yellow()
                    .bold(),
                Span::from(hint).dark_gray(),
            ]));
        }
        let table = Table::new(rows, widths)
            .header(header)
            .block(block)
            .row_highlight_style(
                Style::new()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .column_spacing(1);
        frame.render_stateful_widget(table, area, &mut self.table);
    }

    /// A game name with the part OSDHub doesn't show in yellow
    fn name_line(&self, name: &str) -> Line<'static> {
        let visible = self.screen.visible_chars(name);
        let shown: String = name.chars().take(visible).collect();
        let cut: String = name.chars().skip(visible).collect();
        Line::from(vec![Span::from(shown), Span::from(cut).yellow()])
    }

    fn draw_editor(&self, frame: &mut Frame, editor: &Editor) {
        let [area] = Split::vertical([Constraint::Length(14)])
            .flex(Flex::Center)
            .areas(frame.area());
        let [area] = Split::horizontal([Constraint::Percentage(90)])
            .flex(Flex::Center)
            .areas(area);
        let game = &self.games[editor.game];
        let name: String = editor.name.iter().collect();
        let count = name.trim().chars().count();
        // The leading spaces aren't kept, so the cut is counted from the first character kept
        let leading = name.chars().take_while(|c| *c == ' ').count();
        let visible = leading + self.screen.visible_chars(name.trim());

        // The title ID and the extension can't be edited; the characters OSDHub doesn't show are in yellow
        let mut input = vec![Span::from(editor.parts.prefix.clone()).cyan()];
        for (i, c) in editor.name.iter().enumerate() {
            let mut span = Span::from(c.to_string());
            if i >= visible {
                span = span.yellow();
            }
            if i == editor.cursor {
                span = span.reversed();
            }
            input.push(span);
        }
        if editor.cursor == editor.name.len() {
            input.push(Span::from(" ").reversed());
        }
        input.push(Span::from(editor.parts.ext.clone()).cyan());

        let status = match (
            rename::check_name(editor.parts.console, &name),
            self.screen.warning(name.trim()),
        ) {
            (Err(e), _) => Line::from(format!("✗ {e}")).red(),
            (Ok(()), Some(warning)) => Line::from(format!("⚠ {warning}")).yellow(),
            (Ok(()), None) => {
                Line::from(format!("✓ Fits on OSDHub's menu ({count} characters)")).green()
            }
        };
        // A title ID read from the folder name, not from the disc, is lost with it
        let id_lost = game.id_error.is_some()
            && game
                .id
                .as_ref()
                .is_some_and(|id| crate::disc::find_id(&name).as_ref() != Some(id));
        let error = match &editor.error {
            Some(e) => Line::from(format!("✗ {e}")).red(),
            None if id_lost => Line::from(format!(
                "⚠ The title ID {} comes from the name: without it, the game's art isn't found",
                game.id.as_deref().unwrap_or_default()
            ))
            .yellow(),
            None => Line::from(""),
        };
        // Renaming a PS1 game is up to the user: Ember runs it with any folder name
        let note = match (editor.parts.console, self.screen.fits(&editor.parts.name)) {
            (Console::Ps1, true) => {
                Line::from("Optional: PS1 games can have any name, and this one fits on OSDHub's menu")
            }
            (Console::Ps1, false) => Line::from(
                "Optional: PS1 games can have any name, but the current one doesn't fit on OSDHub's menu",
            )
            .yellow(),
            (Console::Ps2, _) => Line::from("The title ID and the extension stay, as OPL needs them"),
        };
        let covers = if self.screen.covers { "on" } else { "off" };
        let lines = vec![
            note,
            Line::from(format!("Now: {}", rename::file_name(&game.path))).dark_gray(),
            Line::from(""),
            Line::from(input),
            Line::from(""),
            status,
            error,
            Line::from(""),
            Line::from("Enter rename   Tab skip   Esc cancel   ←→ Home End move").bold(),
            Line::from(format!(
                "OSDHub menu_x {}, covers {covers} (--menu-x, --no-covers). OSDHub's favorite and play count of a renamed game start over.",
                self.screen.menu_x
            ))
            .dark_gray(),
        ];
        let what = match editor.parts.console {
            Console::Ps2 => format!("PS2 ISO {}", game.id.as_deref().unwrap_or_default()),
            Console::Ps1 => "PS1 game folder (optional)".to_string(),
        };
        let title = format!(
            " Rename {what} ({} of {}) ",
            editor.position, self.rename_count.1
        );
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(title)),
            area,
        );
    }

    fn draw_log(&self, frame: &mut Frame, area: Rect) {
        let height = area.height.saturating_sub(2) as usize;
        let lines: Vec<Line> = self
            .log
            .iter()
            .skip(self.log.len().saturating_sub(height))
            .map(|l| Line::from(l.as_str()))
            .collect();
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(" Log ")),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn game(console: Console, name: &str, file: &str, id: Option<&str>, subfolder: bool) -> Game {
        Game {
            console,
            path: PathBuf::from("/card/DVD").join(file),
            name: name.to_string(),
            id: id.map(str::to_string),
            id_error: None,
            subfolder,
        }
    }

    /// A device root with a red COV and a blue ICO for SLUS_202.12 in ART/
    fn root_with_art() -> PathBuf {
        let root = std::env::temp_dir().join(format!("osdhub-manager-tui-{}", std::process::id()));
        std::fs::create_dir_all(root.join("ART")).unwrap();
        image::RgbImage::from_pixel(64, 90, image::Rgb([200, 30, 30]))
            .save(root.join("ART/SLUS_202.12_COV.png"))
            .unwrap();
        image::RgbImage::from_pixel(64, 64, image::Rgb([30, 30, 200]))
            .save(root.join("ART/SLUS_202.12_ICO.png"))
            .unwrap();
        root
    }

    fn app() -> App {
        let games = vec![
            game(
                Console::Ps2,
                "Bloody Roar 3",
                "Bloody Roar 3.iso",
                Some("SLUS_202.12"),
                false,
            ),
            game(
                Console::Ps2,
                "Gran Turismo 4",
                "SCUS_973.28.Gran Turismo 4.iso",
                Some("SCUS_973.28"),
                false,
            ),
            game(
                Console::Ps2,
                "Folder Game",
                "Folder Game/game.iso",
                Some("SLES_123.45"),
                true,
            ),
            game(
                Console::Ps1,
                "Crash Bandicoot (USA)",
                "Crash",
                Some("SCUS_949.00"),
                false,
            ),
            game(Console::Ps1, "Unknown", "Unknown", None, false),
        ];
        let art = vec![
            [
                Some("SLUS_202.12_COV.png".into()),
                Some("SLUS_202.12_ICO.png".into()),
            ],
            [Some("missing.png".into()), None],
            [None, None],
            [None, None],
            [None, None],
        ];
        let mut table = TableState::default();
        table.select(Some(0));
        App {
            root: root_with_art(),
            layout: Layout {
                cd_folder: "CD".into(),
                dvd_folder: "DVD".into(),
            },
            sources: Sources {
                oplm_url: None,
                xlenore: false,
            },
            games,
            art,
            filter: Filter::All,
            table,
            marked: BTreeSet::new(),
            types: 0,
            force: false,
            log: vec!["5 games found, 4 with a title ID".into()],
            downloads: None,
            progress: (0, 0),
            screen: Screen {
                menu_x: 400,
                covers: true,
            },
            search: String::new(),
            searching: false,
            tab: Tab::Games,
            config: ConfigTab::new(root_with_art()),
            quit_warned: false,
            editor: None,
            rename_queue: VecDeque::new(),
            rename_count: (0, 0),
            quit: false,
            query_images: false,
            picker: Some(Picker::halfblocks()),
            preview: None,
        }
    }

    fn render(app: &mut App) -> (String, ratatui::buffer::Buffer) {
        render_size(app, 130, 24)
    }

    fn render_size(app: &mut App, width: u16, height: u16) -> (String, ratatui::buffer::Buffer) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        (text, buffer)
    }

    #[test]
    fn screen() {
        let mut app = app();
        let (screen, buffer) = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("SLUS_202.12"));
        // The preview shows the red cover and the blue disc of the selected game as half blocks
        let has = |color: Color| {
            buffer
                .content()
                .iter()
                .any(|c| c.fg == color || c.bg == color)
        };
        assert!(has(Color::Rgb(200, 30, 30)));
        assert!(has(Color::Rgb(30, 30, 200)));
        // The disc is shown at twice its size when there's room: 64 pixels are 6 to 7 cells with half blocks'
        // 10x20 pixel cells
        let (_, buffer) = render_size(&mut app, 130, 50);
        let blue = |y| {
            (0..buffer.area.width)
                .filter(|&x| {
                    let cell = &buffer[(x, y)];
                    cell.fg == Color::Rgb(30, 30, 200) || cell.bg == Color::Rgb(30, 30, 200)
                })
                .count()
        };
        let widest = (0..buffer.area.height).map(blue).max().unwrap();
        println!("disc: {widest} cells wide");
        assert!((12..=14).contains(&widest));
        assert!(screen.contains("rename"));
        assert!(screen.contains("folder"));
        assert!(screen.contains("no ID"));
        assert!(screen.contains("Games (5)"));

        // A missing image is reported in the preview
        app.key(KeyCode::Down);
        let (screen, _) = render(&mut app);
        assert!(screen.contains("SCUS_973.28 ─"));
        assert!(screen.contains("No such file") || screen.contains("cannot find"));
        assert!(screen.contains("no ICO"));
    }

    #[test]
    fn keys() {
        let mut app = app();
        app.key(KeyCode::Char(' ')); // Marks Bloody Roar 3 and moves down
        assert_eq!(app.marked.iter().copied().collect::<Vec<_>>(), vec![0]);
        assert_eq!(app.table.selected(), Some(1));
        app.key(KeyCode::Tab); // PS2 only
        assert_eq!(app.visible(), vec![0, 1, 2]);
        app.key(KeyCode::Tab); // PS1 only: nothing marked there, so the targets are the PS1 games
        assert_eq!(app.targets(), vec![3, 4]);
        app.key(KeyCode::Tab);
        app.key(KeyCode::Char('t'));
        assert_eq!(TYPE_CHOICES[app.types], &[ArtType::Cov]);

        // Renaming the marked game opens the editor, and Esc cancels
        app.key(KeyCode::Char('r'));
        let editor = app.editor.as_ref().unwrap();
        assert_eq!(editor.game, 0);
        assert_eq!(editor.name.iter().collect::<String>(), "Bloody Roar 3");
        let (screen, _) = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("Rename PS2 ISO SLUS_202.12 (1 of 1)"));
        assert!(screen.contains("SLUS_202.12.Bloody Roar 3 .iso"));
        assert!(screen.contains("Fits on OSDHub"));
        app.key(KeyCode::Char('q')); // Typed into the name
        assert!(!app.quit);
        app.key(KeyCode::Esc);
        assert!(app.editor.is_none());
        app.key(KeyCode::Char('q'));
        assert!(app.quit);
    }

    #[test]
    fn search() {
        let mut app = app();
        let (screen, _) = render(&mut app);
        assert!(screen.contains("SLUS_202.12 USA"));
        assert!(screen.contains("no ID       -"));

        // `/` types the search, where every key is text (q doesn't quit), in any order
        app.key(KeyCode::Char('/'));
        for c in "ps1 crash q".chars() {
            app.key(KeyCode::Char(c));
        }
        assert!(!app.quit);
        assert!(app.visible().is_empty());
        app.key(KeyCode::Backspace);
        app.key(KeyCode::Backspace);
        assert_eq!(app.visible(), vec![3]);
        let (screen, _) = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("/ ps1 crash█"));
        assert!(screen.contains("Games (1)"));

        // Enter keeps the search, while the keys work again; Esc clears it, and then quits
        app.key(KeyCode::Enter);
        assert!(!app.searching);
        assert_eq!(app.selected(), Some(3));
        app.key(KeyCode::Esc);
        assert_eq!(app.visible().len(), 5);
        assert!(!app.quit);

        // A region, and the letters of a name in order
        app.key(KeyCode::Char('/'));
        for c in "usa grtur".chars() {
            app.key(KeyCode::Char(c));
        }
        assert_eq!(app.visible(), vec![1]);
        app.key(KeyCode::Esc);
        app.key(KeyCode::Esc);
        assert!(app.quit);
    }

    #[test]
    fn rename() {
        let dir =
            std::env::temp_dir().join(format!("osdhub-manager-tui-rename-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let long = "HARVEST MOON - SAVE THE HOMELAND";
        std::fs::write(dir.join(format!("{long}.iso")), b"").unwrap();
        let mut app = app();
        app.games[0] = Game {
            path: dir.join(format!("{long}.iso")),
            name: long.to_string(),
            ..app.games[0].clone()
        };
        // A PS1 game whose name doesn't fit, and whose title ID comes from the folder name
        let ps1 = "Crash Bandicoot - The Wrath of Cortex (SLUS_013.92)";
        std::fs::create_dir_all(dir.join(ps1)).unwrap();
        app.games[3] = Game {
            path: dir.join(ps1),
            name: ps1.to_string(),
            id: Some("SLUS_013.92".to_string()),
            id_error: Some("no SYSTEM.CNF".to_string()),
            ..app.games[3].clone()
        };
        app.marked.extend([0, 2, 3]);

        // The names are too long for OSDHub with covers: the Rename column warns, and the cut part is in yellow,
        // in the table and the editor
        let (screen, buffer) = render(&mut app);
        assert!(screen.contains("rename ⚠ HARVEST MOON"));
        assert!(screen.contains("⚠        Crash Bandicoot - The Wrath"));
        let row = (0..buffer.area.height)
            .find(|&y| {
                let line: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                line.contains(long)
            })
            .unwrap();
        let line: Vec<&str> = (0..buffer.area.width)
            .map(|x| buffer[(x, row)].symbol())
            .collect();
        let start = (0..line.len())
            .find(|&x| line[x..].concat().starts_with(long))
            .unwrap() as u16;
        assert_ne!(buffer[(start, row)].fg, Color::Yellow);
        assert_eq!(
            buffer[(start + long.len() as u16 - 1, row)].fg,
            Color::Yellow
        );
        app.key(KeyCode::Char('r'));
        let (screen, _) = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("Rename PS2 ISO SLUS_202.12 (1 of 3)"));
        assert!(screen.contains("⚠ OSDHub shows \"HARVEST MOON"));

        // Only the name is edited: the title ID and the extension stay
        app.key(KeyCode::Home);
        for _ in 0.."HARVEST MOON - ".len() {
            app.key(KeyCode::Delete);
        }
        app.key(KeyCode::End);
        for c in " - The Legend of Harvest Moon".chars() {
            app.key(KeyCode::Char(c));
        }
        let (screen, _) = render(&mut app);
        assert!(screen.contains("SLUS_202.12.SAVE THE HOMELAND - The Legend of Harvest Moon .iso"));
        assert!(screen.contains("⚠"));
        for _ in 0.." - The Legend of Harvest Moon".len() {
            app.key(KeyCode::Backspace);
        }
        let (screen, _) = render(&mut app);
        assert!(screen.contains("✓ Fits on OSDHub's menu (17 characters)"));

        // An invalid name isn't renamed
        app.key(KeyCode::Char('?'));
        app.key(KeyCode::Enter);
        assert!(app.editor.as_ref().unwrap().error.is_some());
        app.key(KeyCode::Backspace);

        // Enter renames and goes on to the next game: the folder game can't be renamed, so the PS1 game is next
        app.key(KeyCode::Enter);
        assert!(dir.join("SLUS_202.12.SAVE THE HOMELAND.iso").exists());
        assert_eq!(app.games[0].name, "SAVE THE HOMELAND");
        assert!(
            app.log
                .iter()
                .any(|l| l.contains("Folder Game: not renamed"))
        );

        // Renaming the PS1 game is optional, but its name doesn't fit; the whole folder name is edited
        let (screen, _) = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("Rename PS1 game folder (optional) (3 of 3)"));
        assert!(
            screen
                .contains("Optional: PS1 games can have any name, but the current one doesn't fit")
        );
        for _ in 0.." - The Wrath of Cortex (SLUS_013.92)".len() {
            app.key(KeyCode::Backspace);
        }
        let (screen, _) = render(&mut app);
        assert!(screen.contains("⚠ The title ID SLUS_013.92 comes from the name"));
        assert!(screen.contains("✓ Fits on OSDHub's menu (15 characters)"));
        app.key(KeyCode::Enter);
        assert!(app.editor.is_none());
        assert!(dir.join("Crash Bandicoot").is_dir());
        assert_eq!(app.games[3].name, "Crash Bandicoot");
        assert!(app.log.iter().any(|l| l.contains("2 game(s) renamed")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
