//! Terminal interface: the games of the device in a table, with their title IDs, art and OPL names,
//! where the art is downloaded and the PS2 ISOs are renamed. Downloads run in a separate thread,
//! reporting to the log at the bottom, so the interface keeps responding.
//! The art of the selected game is previewed on the right, as an image in terminals that show images
//! (kitty's protocol, Sixel or iTerm2's), or with colored half blocks in the others.

use crate::covers::{self, ArtType, Downloader, Outcome, Sources};
use crate::games::{self, Console, Game, Layout};
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
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
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
    /// Renames waiting for confirmation
    confirm: Option<Vec<(PathBuf, PathBuf)>>,
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

/// The preview is only shown when the terminal is at least this wide
const PREVIEW_MIN_WIDTH: u16 = 100;
const PREVIEW_WIDTH: u16 = 32;

pub fn run(
    root: PathBuf,
    layout: Layout,
    sources: Sources,
    consoles: &[Console],
    query_images: bool,
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
        confirm: None,
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
        if let Some(renames) = self.confirm.take() {
            if matches!(code, KeyCode::Char('y') | KeyCode::Enter) {
                self.apply_renames(renames);
            } else {
                self.log("Rename cancelled".to_string());
            }
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
            KeyCode::Char('r') => self.plan_renames(),
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

    fn plan_renames(&mut self) {
        let mut renames = Vec::new();
        for i in self.targets() {
            if self.games[i].console != Console::Ps2 {
                continue;
            }
            match rename::plan(&self.games[i]) {
                Plan::Rename { from, to } => renames.push((from, to)),
                Plan::Skip(reason) => {
                    let line = format!("{}: not renamed, {reason}", self.games[i].name);
                    self.log(line);
                }
                Plan::AlreadyNamed => {}
            }
        }
        if renames.is_empty() {
            self.log("No PS2 ISOs to rename".to_string());
        } else {
            self.confirm = Some(renames);
        }
    }

    fn apply_renames(&mut self, renames: Vec<(PathBuf, PathBuf)>) {
        let mut renamed = 0;
        for (from, to) in &renames {
            match rename::apply(from, to) {
                Ok(()) => renamed += 1,
                Err(e) => self.log(format!("{}: {e}", from.display())),
            }
        }
        self.log(format!(
            "{renamed} ISO(s) renamed. Refresh the Games list in OSDHub, since the paths changed."
        ));
        self.rescan();
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
                "↑↓ move  Space mark  a mark all  Tab PS2/PS1  c download art  t types  f force  r rename PS2 ISOs  s rescan  q quit",
            )
            .dark_gray(),
            help,
        );
        if let Some(renames) = &self.confirm {
            draw_confirm(frame, renames);
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
                    Cell::from(game.name.clone()),
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

fn draw_confirm(frame: &mut Frame, renames: &[(PathBuf, PathBuf)]) {
    let height = (renames.len() as u16 + 4).min(frame.area().height.saturating_sub(2));
    let [area] = Split::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(frame.area());
    let [area] = Split::horizontal([Constraint::Percentage(90)])
        .flex(Flex::Center)
        .areas(area);
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let mut lines: Vec<Line> = renames
        .iter()
        .map(|(from, to)| Line::from(format!("{} → {}", name(from), name(to))))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from("Enter/y rename   any other key cancels").bold());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(format!(
            " Rename {} PS2 ISO(s) to OPL's names? ",
            renames.len()
        ))),
        area,
    );
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
            confirm: None,
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

        // Renaming the marked game asks first, and any other key cancels
        app.key(KeyCode::Char('r'));
        let renames = app.confirm.clone().unwrap();
        assert_eq!(renames.len(), 1);
        assert!(renames[0].1.ends_with("SLUS_202.12.Bloody Roar 3.iso"));
        let (screen, _) = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("Rename 1 PS2 ISO(s)"));
        app.key(KeyCode::Char('n'));
        assert!(app.confirm.is_none());
        app.key(KeyCode::Char('q'));
        assert!(app.quit);
    }
}
