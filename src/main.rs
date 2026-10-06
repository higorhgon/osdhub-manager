//! osdhub-manager: manages the games of an OSDHub device from a computer
//! (the SD card of an MMCE device, a USB drive...), without OPL Manager.

mod covers;
mod disc;
mod games;
mod osdhub;
mod rename;
mod tui;

use covers::{ArtType, Downloader, Outcome, Sources};
use games::{Console, Layout};
use osdhub::Screen;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
osdhub-manager - manages the games of an OSDHub device from a computer

USAGE:
    osdhub-manager <DEVICE ROOT> [OPTIONS]            Opens the terminal interface
    osdhub-manager <COMMAND> <DEVICE ROOT> [OPTIONS]

COMMANDS:
    tui       Opens the terminal interface (the default)
    list      Lists the games with the title IDs read from their discs
    covers    Downloads the case covers and discs of the games into ART/ (<ID>_COV.jpg, <ID>_ICO.png...)
    rename    Renames the PS2 ISOs to OPL's <ID>.<name>.iso form (only shows the changes without --apply)

OPTIONS:
    --ps1               Only PS1 games (EMBER/games/<game>/ with a .cue file)
    --ps2               Only PS2 games (ISOs in the CD and DVD folders)
    --types <LIST>      Art types to download, comma-separated: cov, ico (default: cov,ico)
    --force             Downloads the images again even when they're already in ART/
    --dry-run           Shows what would be downloaded without downloading
    --apply             Renames the ISOs (rename)
    --cd-folder <DIR>   PS2 CD folder (default: CD), like games_cd_folder
    --dvd-folder <DIR>  PS2 DVD folder (default: DVD), like games_dvd_folder
    --oplm-url <URL>    OPL Manager art database (default: the archive.org backup), \"none\" to skip it
    --no-xlenore        Doesn't use xlenore's cover collections
    --no-images         Draws the art previews with colored half blocks instead of asking the terminal for images
    --menu-x <N>        Center of OSDHub's menu, like OSDSYS_menu_x (default: 400), for the names that don't fit
    --no-covers         OSDHub doesn't show covers (games_covers = 0), which leaves more room for the names
    -h, --help          Shows this help
    -V, --version       Shows the version

EXAMPLES:
    osdhub-manager /run/media/$USER/MMCE
    osdhub-manager list /run/media/$USER/MMCE
    osdhub-manager covers /run/media/$USER/MMCE --ps1 --types cov
    osdhub-manager rename /run/media/$USER/MMCE --apply
";

struct Options {
    command: String,
    root: PathBuf,
    consoles: Vec<Console>,
    types: Vec<ArtType>,
    force: bool,
    dry_run: bool,
    apply: bool,
    layout: Layout,
    oplm_url: Option<String>,
    xlenore: bool,
    images: bool,
    screen: Screen,
}

fn parse_args() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let mut positional = Vec::new();
    let (mut ps1, mut ps2) = (false, false);
    let mut options = Options {
        command: String::new(),
        root: PathBuf::new(),
        consoles: Vec::new(),
        types: vec![ArtType::Cov, ArtType::Ico],
        force: false,
        dry_run: false,
        apply: false,
        layout: Layout {
            cd_folder: "CD".to_string(),
            dvd_folder: "DVD".to_string(),
        },
        oplm_url: Some(covers::DEFAULT_OPLM_URL.to_string()),
        xlenore: true,
        images: true,
        screen: Screen {
            menu_x: 400,
            covers: true,
        },
    };

    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("osdhub-manager {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--ps1" => ps1 = true,
            "--ps2" => ps2 = true,
            "--force" => options.force = true,
            "--dry-run" => options.dry_run = true,
            "--apply" => options.apply = true,
            "--no-xlenore" => options.xlenore = false,
            "--no-images" => options.images = false,
            "--no-covers" => options.screen.covers = false,
            "--menu-x" => {
                let x = value("--menu-x")?;
                options.screen.menu_x = x
                    .parse()
                    .ok()
                    .filter(|x| (0..=640).contains(x))
                    .ok_or(format!("--menu-x needs a number from 0 to 640, not {x}"))?;
            }
            "--types" => {
                let list = value("--types")?;
                options.types = list
                    .split(',')
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| {
                        ArtType::parse(t.trim())
                            .ok_or(format!("unknown art type: {t} (use cov or ico)"))
                    })
                    .collect::<Result<_, _>>()?;
            }
            "--cd-folder" => options.layout.cd_folder = value("--cd-folder")?,
            "--dvd-folder" => options.layout.dvd_folder = value("--dvd-folder")?,
            "--oplm-url" => {
                let url = value("--oplm-url")?;
                options.oplm_url = if url.eq_ignore_ascii_case("none") {
                    None
                } else {
                    Some(url)
                };
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            _ => positional.push(arg),
        }
    }

    let (command, root) = match positional.as_slice() {
        [root] => ("tui", root),
        [command, root] => (command.as_str(), root),
        _ => {
            return Err("expected the device root, with an optional command before it".to_string());
        }
    };
    options.command = command.to_string();
    options.root = PathBuf::from(root);
    if !options.root.is_dir() {
        return Err(format!("{} is not a folder", options.root.display()));
    }
    if !ps1 && !ps2 {
        ps1 = true;
        ps2 = true;
    }
    if ps2 {
        options.consoles.push(Console::Ps2);
    }
    if ps1 {
        options.consoles.push(Console::Ps1);
    }
    Ok(options)
}

fn list(options: &Options) -> ExitCode {
    let games = games::scan(&options.root, &options.layout, &options.consoles);
    for game in &games {
        let id = game.id.as_deref().unwrap_or("-----------");
        let note = match (&game.id, &game.id_error) {
            (Some(_), Some(_)) => " (ID from the file name)".to_string(),
            (None, Some(e)) => format!(" ({e})"),
            _ => String::new(),
        };
        println!("{}  {id}  {}{note}", game.console, game.name);
        if let Some(warning) = options.screen.warning(&game.name) {
            println!("     ⚠ {warning}");
        }
    }
    println!("{} game(s)", games.len());
    ExitCode::SUCCESS
}

fn download_covers(options: &Options) -> ExitCode {
    let games = games::scan(&options.root, &options.layout, &options.consoles);
    let art_dir = options.root.join("ART");
    let downloader = Downloader::new(Sources {
        oplm_url: options.oplm_url.clone(),
        xlenore: options.xlenore,
    });
    let (mut downloaded, mut existing, mut missing, mut failed) = (0, 0, 0, 0);

    for game in &games {
        let Some(id) = &game.id else {
            println!("{}  {}: no title ID, skipped", game.console, game.name);
            missing += options.types.len();
            continue;
        };
        for &art in &options.types {
            let result = match downloader.download(
                game,
                id,
                art,
                &art_dir,
                options.force,
                options.dry_run,
            ) {
                Outcome::Exists(file) => {
                    existing += 1;
                    format!("already in ART/ ({file})")
                }
                Outcome::Downloaded { file, source } => {
                    downloaded += 1;
                    format!("{file} ({source})")
                }
                Outcome::WouldDownload => {
                    downloaded += 1;
                    "would be downloaded".to_string()
                }
                Outcome::NotFound => {
                    missing += 1;
                    "not found".to_string()
                }
                Outcome::Failed(e) => {
                    failed += 1;
                    format!("failed: {e}")
                }
            };
            println!(
                "{}  {id}  {} {}: {result}",
                game.console,
                art.suffix(),
                game.name
            );
        }
    }

    let verb = if options.dry_run {
        "to download"
    } else {
        "downloaded"
    };
    println!("{downloaded} {verb}, {existing} already there, {missing} not found, {failed} failed");
    if downloaded > 0 && !options.dry_run {
        println!("Refresh the game lists in OSDHub (games_covers = 1) to convert the new images.");
    }
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn rename_isos(options: &Options) -> ExitCode {
    let games = games::scan(&options.root, &options.layout, &[Console::Ps2]);
    let (mut renamed, mut failed, mut cut) = (0, 0, 0);
    for game in &games {
        if let Some(warning) = options.screen.warning(&game.name) {
            println!("warn  {}: {warning}", game.path.display());
            cut += 1;
        }
        match rename::plan(game) {
            rename::Plan::AlreadyNamed => {}
            rename::Plan::Skip(reason) => println!("skip  {}: {reason}", game.path.display()),
            rename::Plan::Rename { from, to } => {
                let to_name = rename::file_name(&to);
                if !options.apply {
                    println!("would rename  {} -> {to_name}", from.display());
                    renamed += 1;
                    continue;
                }
                match rename::apply(&from, &to) {
                    Ok(()) => {
                        println!("renamed  {} -> {to_name}", from.display());
                        renamed += 1;
                    }
                    Err(e) => {
                        println!("failed  {}: {e}", from.display());
                        failed += 1;
                    }
                }
            }
        }
    }
    if options.apply {
        println!("{renamed} renamed, {failed} failed");
        if renamed > 0 {
            println!("Refresh the Games list in OSDHub, since the ISO paths changed.");
        }
    } else {
        println!("{renamed} to rename. Run again with --apply to rename them.");
    }
    if cut > 0 {
        println!(
            "{cut} name(s) don't fit on OSDHub's menu: edit them with r in the terminal interface."
        );
    }
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn main() -> ExitCode {
    let options = match parse_args() {
        Ok(options) => options,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match options.command.as_str() {
        "tui" => {
            let sources = Sources {
                oplm_url: options.oplm_url.clone(),
                xlenore: options.xlenore,
            };
            match tui::run(
                options.root.clone(),
                options.layout,
                sources,
                &options.consoles,
                options.images,
                options.screen,
            ) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "list" => list(&options),
        "covers" => download_covers(&options),
        "rename" => rename_isos(&options),
        other => {
            eprintln!("error: unknown command: {other}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
