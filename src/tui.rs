//! Terminal interface: the games of the device in a table, with their title IDs, art and OPL names,
//! where the art is downloaded and the PS2 ISOs are renamed. Downloads run in a separate thread,
//! reporting to the log at the bottom, so the interface keeps responding.
//! The art of the selected game is previewed on the right, as an image in terminals that show images
//! (kitty's protocol, Sixel or iTerm2's), or with colored half blocks in the others.
//! The names that don't fit on OSDHub's menu are shown with the part it cuts in yellow, and renaming an ISO
//! opens an editor for its name, which warns when the name is too long for OSDHub.

use crate::covers::{self, ArtType, Downloader, Outcome, Sources};
use crate::games::{self, Console, Game, Layout};
use crate::osdhub::Screen;
use crate::rename::{self, Plan};
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

/// Edits the name of a PS2 ISO, between its title ID and its extension, which stay
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

pub fn run(
    root: PathBuf,
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
    let mut app = App {
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
        editor: None,
        rename_queue: VecDeque::new(),
        rename_count: (0, 0),
        quit: false,
        query_images,
        picker: None,
        preview: None,
    };
    println!("Reading the games on {}...", app.root.display());
    app.rescan();
    ratatui::run(|terminal| app.main_loop(terminal))
}

impl App {
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

    /// Indices of the games shown with the current filter
    fn visible(&self) -> Vec<usize> {
        (0..self.games.len())
            .filter(|&i| self.filter.shows(self.games[i].console))
            .collect()
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

    fn key(&mut self, code: KeyCode) {
        if self.editor.is_some() {
            self.edit(code);
            return;
        }
        let rows = self.visible().len();
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
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

    /// Opens the name editor for the marked PS2 ISOs, one after the other, or for the selected one
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
        self.rename_queue = games
            .into_iter()
            .filter(|&i| self.games[i].console == Console::Ps2)
            .collect();
        if self.rename_queue.is_empty() {
            self.log("Select or mark the PS2 ISOs to rename".to_string());
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
                "{renamed} ISO(s) renamed. Refresh the Games list in OSDHub, since the paths changed."
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
                game.name = games::display_name(&rename::file_name(&to));
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
                "↑↓ move  Space mark  a mark all  Tab PS2/PS1  c download art  t types  f force  r rename PS2 ISO  s rescan  q quit",
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
        let types: Vec<&str> = TYPE_CHOICES[self.types]
            .iter()
            .map(|t| t.suffix())
            .collect();
        let mut status = vec![
            tab("All", Filter::All),
            tab("PS2", Filter::Ps2),
            tab("PS1", Filter::Ps1),
            Span::from(format!("   Art: {}", types.join("+"))),
            Span::from(if self.force {
                "   Force: on"
            } else {
                "   Force: off"
            }),
            Span::from(format!("   Marked: {}", self.marked.len())),
        ];
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
                    // a small image that breaks up when enlarged, keeps its size unless it doesn't fit
                    let resize = if label == "ICO" {
                        Resize::Fit(Some(FilterType::Triangle))
                    } else {
                        Resize::Scale(Some(FilterType::Triangle))
                    };
                    let size = protocol.size_for(resize.clone(), area.as_size());
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
                let opl = match (game.console, rename::plan(game)) {
                    (Console::Ps1, _) => Cell::from("-").dark_gray(),
                    (_, Plan::AlreadyNamed) => Cell::from("✓").green(),
                    (_, Plan::Rename { .. }) => Cell::from("rename").yellow(),
                    (_, Plan::Skip(_)) if game.subfolder => Cell::from("folder").dark_gray(),
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
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Fill(1),
        ];
        let header = Row::new(["", "", "Title ID", "COV", "ICO", "OPL", "Name"]).bold();
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::bordered().title(format!(" Games ({}) ", self.visible().len())))
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
        let [area] = Split::vertical([Constraint::Length(12)])
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
        let mut input = vec![Span::from(format!("{}.", editor.parts.id)).cyan()];
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

        let status = match (rename::check_name(&name), self.screen.warning(name.trim())) {
            (Err(e), _) => Line::from(format!("✗ {e}")).red(),
            (Ok(()), Some(warning)) => Line::from(format!("⚠ {warning}")).yellow(),
            (Ok(()), None) => {
                Line::from(format!("✓ Fits on OSDHub's menu ({count} characters)")).green()
            }
        };
        let error = match &editor.error {
            Some(e) => Line::from(format!("✗ {e}")).red(),
            None => Line::from(""),
        };
        let covers = if self.screen.covers { "on" } else { "off" };
        let lines = vec![
            Line::from(format!("Now: {}", rename::file_name(&game.path))).dark_gray(),
            Line::from(""),
            Line::from(input),
            Line::from(""),
            status,
            error,
            Line::from(""),
            Line::from("Enter rename   Tab skip   Esc cancel   ←→ Home End move").bold(),
            Line::from(format!(
                "OSDHub menu_x {}, covers {covers} (--menu-x, --no-covers)",
                self.screen.menu_x
            ))
            .dark_gray(),
        ];
        let title = format!(
            " Rename {} ({} of {}) ",
            editor.parts.id, editor.position, self.rename_count.1
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
        let mut terminal = Terminal::new(TestBackend::new(130, 24)).unwrap();
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
        assert!(screen.contains("Rename SLUS_202.12 (1 of 1)"));
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
        app.marked.extend([0, 2, 3]);

        // The name is too long for OSDHub with covers: the cut part is in yellow, in the table and the editor
        let (_, buffer) = render(&mut app);
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
        assert!(screen.contains("1 of 2"));
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

        // Enter renames and goes on to the next game: the folder game can't be renamed (and the PS1 game
        // isn't one of them), so the renames end
        app.key(KeyCode::Enter);
        assert!(app.editor.is_none());
        assert!(dir.join("SLUS_202.12.SAVE THE HOMELAND.iso").exists());
        assert_eq!(app.games[0].name, "SAVE THE HOMELAND");
        assert!(
            app.log
                .iter()
                .any(|l| l.contains("Folder Game: not renamed"))
        );
        assert!(app.log.iter().any(|l| l.contains("1 ISO(s) renamed")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
