//! osdhub-manager: manages the games of an OSDHub device from a computer
//! (the SD card of an MMCE device, a USB drive...), without OPL Manager.

mod browse;
mod cheats;
mod cnf;
mod config;
mod config_tab;
mod covers;
mod disc;
mod games;
mod install;
mod install_modal;
mod memcard;
mod net;
mod osdhub;
mod rename;
mod search;
mod tui;

use covers::{ArtType, Downloader, Outcome, Sources};
use games::{Console, Layout};
use osdhub::Screen;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
osdhub-manager - manages the games of an OSDHub device from a computer

USAGE:
    osdhub-manager [DEVICE ROOT] [OPTIONS]            Opens the terminal interface, choosing the device
                                                      root in a folder browser when it isn't given
    osdhub-manager <COMMAND> <DEVICE ROOT> [OPTIONS]

COMMANDS:
    tui       Opens the terminal interface (the default)
    list      Lists the games with the title IDs read from their discs
    covers    Downloads the case covers and discs of the games into ART/ (<ID>_COV.jpg, <ID>_ICO.png...)
    rename    Renames the PS2 ISOs to OPL's <ID>.<name>.iso form (only shows the changes without --apply)
    install   Installs OSDHub as mc0:/BOOT/BOOT.ELF in the device's BOOT memory card (or in a folder to copy to
              a memory card), and RiptOPL, Neutrino and Ember on the device, from their latest releases, after
              showing what's written and asking
    config    Shows OSDMenu's configuration (SYS-CONF/OSDMENU.CNF inside the device's BOOT memory card, or the
              memory card image or .cnf file given instead of the device root), exports it or imports it

OPTIONS:
    --ps1               Only PS1 games (EMBER/games/<game>/ with a .cue file)
    --ps2               Only PS2 games (ISOs in the CD and DVD folders)
    --types <LIST>      Art types to download, comma-separated: cov, ico (default: cov,ico)
    --force             Downloads the images again even when they're already in ART/
    --dry-run           Shows what would be downloaded without downloading
    --apply             Renames the ISOs (rename)
    --cd-folder <DIR>   PS2 CD folder (default: CD), like games_cd_folder
    --dvd-folder <DIR>  PS2 DVD folder (default: DVD), like games_dvd_folder
    --oplm-url <URL>    OPL Manager art database (default: its dump on GitHub), \"none\" to skip it
    --no-xlenore        Doesn't use xlenore's cover collections
    --no-images         Draws the art previews with colored half blocks instead of asking the terminal for images
    --menu-x <N>        Center of OSDHub's menu, like OSDSYS_menu_x (default: 400), for the names that don't fit
    --no-covers         OSDHub doesn't show covers (games_covers = 0), which leaves more room for the names
    --export <FILE>     Writes the configuration to FILE, to edit it (config)
    --import <FILE>     Saves FILE as the configuration, after showing the changes and asking (config)
    --yes               Doesn't ask before saving (config --import, install)
    --opl / --opl-ra    Installs RiptOPL / its RetroAchievements build (install)
    --neutrino          Installs Neutrino (install)
    --ember             Installs Ember, a beta by Gageformer (install)
    --bios <FILE>       PS1 BIOS dumped from your console, copied to EMBER/bios.bin (install)
    --card <FILE>       Memory card image to install OSDHub in (install; default: the only BOOT one)
    --folder <DIR>      Folder to put OSDHub's BOOT and SYS-CONF in, to copy them to a memory card (install;
                        the default without a BOOT memory card image: OSDHUB-MC on the device)
    -h, --help          Shows this help
    -V, --version       Shows the version

EXAMPLES:
    osdhub-manager /run/media/$USER/MMCE
    osdhub-manager list /run/media/$USER/MMCE
    osdhub-manager covers /run/media/$USER/MMCE --ps1 --types cov
    osdhub-manager rename /run/media/$USER/MMCE --apply
    osdhub-manager config /run/media/$USER/MMCE --export OSDMENU.CNF
    osdhub-manager config /run/media/$USER/MMCE --import OSDMENU.CNF
";

struct Options {
    command: String,
    /// None to pick it in the terminal interface
    root: Option<PathBuf>,
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
    export: Option<PathBuf>,
    import: Option<PathBuf>,
    yes: bool,
    install: install::Choice,
    card: Option<PathBuf>,
    folder: Option<PathBuf>,
}

fn parse_args() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let mut positional = Vec::new();
    let (mut ps1, mut ps2) = (false, false);
    let mut options = Options {
        command: String::new(),
        root: None,
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
        export: None,
        import: None,
        yes: false,
        install: install::Choice::default(),
        card: None,
        folder: None,
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
            "--yes" | "-y" => options.yes = true,
            "--opl" => options.install.opl = true,
            "--opl-ra" => {
                options.install.opl = true;
                options.install.opl_ra = true;
            }
            "--neutrino" => options.install.neutrino = true,
            "--ember" => options.install.ember = true,
            "--bios" => {
                options.install.ember = true;
                options.install.bios = Some(PathBuf::from(value("--bios")?));
            }
            "--card" => options.card = Some(PathBuf::from(value("--card")?)),
            "--folder" => options.folder = Some(PathBuf::from(value("--folder")?)),
            "--export" => options.export = Some(PathBuf::from(value("--export")?)),
            "--import" => options.import = Some(PathBuf::from(value("--import")?)),
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

    const COMMANDS: [&str; 6] = ["tui", "list", "covers", "rename", "config", "install"];
    let (command, root) = match positional.as_slice() {
        [] => ("tui", None),
        [command] if COMMANDS.contains(&command.as_str()) && !Path::new(command).is_dir() => {
            (command.as_str(), None)
        }
        [root] => ("tui", Some(root)),
        [command, root] => (command.as_str(), Some(root)),
        _ => {
            return Err("expected the device root, with an optional command before it".to_string());
        }
    };
    options.command = command.to_string();
    if let Some(root) = root {
        let root = PathBuf::from(root);
        // config also takes a memory card image or a .cnf file
        if !(root.is_dir() || command == "config" && root.is_file()) {
            return Err(format!("{} is not a folder", root.display()));
        }
        options.root = Some(root);
    } else if command != "tui" {
        return Err(format!("{command} needs the device root"));
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

/// The device root of the commands, which parse_args() requires
fn device_root(options: &Options) -> &Path {
    options
        .root
        .as_deref()
        .expect("the commands need the device root")
}

fn list(options: &Options) -> ExitCode {
    let games = games::scan(device_root(options), &options.layout, &options.consoles);
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
    let games = games::scan(device_root(options), &options.layout, &options.consoles);
    let art_dir = device_root(options).join("ART");
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
    let games = games::scan(device_root(options), &options.layout, &[Console::Ps2]);
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

/// Where the configuration is: the file given, or the BOOT memory card of the device with OSDMENU.CNF
/// (or the only one, where it's created)
fn config_source(root: &Path) -> Result<config::Source, String> {
    if root.is_file() {
        return Ok(config::Source::of(root));
    }
    let cards = config::find_cards(root);
    if let Some(card) = cards.iter().find(|c| c.cnf == Ok(true)) {
        return Ok(config::Source::Card(card.path.clone()));
    }
    let readable: Vec<_> = cards.iter().filter(|c| c.cnf.is_ok()).collect();
    match readable.as_slice() {
        [card] => Ok(config::Source::Card(card.path.clone())),
        [] => Err(format!(
            "no memory card image in {}/MemoryCards/**/BOOT/; give the image or the .cnf file instead",
            root.display()
        )),
        _ => Err(
            "more than one BOOT memory card without OSDMENU.CNF; give the image instead"
                .to_string(),
        ),
    }
}

fn show_config(options: &Options) -> ExitCode {
    let source = match config_source(device_root(options)) {
        Ok(source) => source,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let original = match config::load(&source) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let Some(import) = &options.import else {
        if let Some(export) = &options.export {
            return match std::fs::write(export, &original) {
                Ok(()) => {
                    println!("{} written from {}", export.display(), source.describe());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {}: {e}", export.display());
                    ExitCode::FAILURE
                }
            };
        }
        println!("# {}", source.describe());
        print!("{original}");
        for (line, problem) in cnf::problems(&cnf::Cnf::parse(&original)) {
            eprintln!("warning: line {}: {problem}", line + 1);
        }
        return ExitCode::SUCCESS;
    };

    let new = match std::fs::read(import).map(String::from_utf8) {
        Ok(Ok(text)) => text,
        Ok(Err(_)) => {
            eprintln!("error: {} isn't UTF-8 text", import.display());
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("error: {}: {e}", import.display());
            return ExitCode::FAILURE;
        }
    };
    let changes = cnf::diff(&original, &new);
    if changes.is_empty() {
        println!(
            "{} is the same as the configuration in {}",
            import.display(),
            source.describe()
        );
        return ExitCode::SUCCESS;
    }
    println!("Changes to {}:", source.describe());
    for (old, new) in &changes {
        if let Some(old) = old {
            println!("  - {old}");
        }
        if let Some(new) = new {
            println!("  + {new}");
        }
    }
    for (line, problem) in cnf::problems(&cnf::Cnf::parse(&new)) {
        println!("warning: line {}: {problem}", line + 1);
    }
    if !options.yes {
        print!(
            "Save them? A copy of {} is made first. [y/N] ",
            source.path().display()
        );
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("Not saved.");
            return ExitCode::SUCCESS;
        }
    }
    match config::save(&source, &original, &new) {
        Ok(backup) => {
            println!("Saved. The previous version is in {}", backup.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn install_apps(options: &Options) -> ExitCode {
    let root = device_root(options);
    let target = match (&options.folder, &options.card) {
        (Some(folder), _) => install::Target::Folder(folder.clone()),
        (None, Some(card)) => install::Target::Card(card.clone()),
        (None, None) => match install_modal::card_for(None, &config::find_cards(root)) {
            Some(card) => install::Target::Card(card),
            None => {
                let folder = root.join(install::FOLDER);
                println!(
                    "No BOOT memory card image in {}/MemoryCards: OSDHub goes in {}, to copy to a memory card",
                    root.display(),
                    folder.display()
                );
                install::Target::Folder(folder)
            }
        },
    };
    let plan = match install::prepare(root, &target, &options.install, &|line| println!("{line}")) {
        Ok(plan) => plan,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!();
    for source in &plan.sources {
        println!("{source}");
    }
    println!("Files:");
    for (file, replaces) in plan.describe(root) {
        println!("  {file}{}", if replaces { "   (replaces it)" } else { "" });
    }
    if plan.creates_cnf {
        println!(
            "There's no OSDMENU.CNF there: it gets the example (edit it with the config command)"
        );
    }
    for note in &plan.notes {
        println!("warning: {note}");
    }
    if !options.yes {
        match &target {
            install::Target::Card(card) => {
                print!(
                    "Install? A copy of {} is made first. [y/N] ",
                    card.display()
                )
            }
            install::Target::Folder(_) => print!("Install? [y/N] "),
        }
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("Not installed.");
            return ExitCode::SUCCESS;
        }
    }
    match plan.apply(root) {
        Ok(log) => {
            for line in log {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
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
        "config" => show_config(&options),
        "install" => install_apps(&options),
        other => {
            eprintln!("error: unknown command: {other}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
