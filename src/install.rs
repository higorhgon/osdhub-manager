//! Installs OSDHub and the programs it launches on a device, from their latest releases on GitHub:
//!
//! - **OSDHub** (always): `osdmenu.elf` from higorhgon/osdmenu's release package becomes `BOOT/BOOT.ELF` in the
//!   memory card image an MMCE device boots from, or in a folder to copy to a memory card (from a USB drive, with
//!   wLaunchELF), which also gets the example `SYS-CONF/OSDMENU.CNF` and the `SYS-CONF` icons when it has none
//! - **RiptOPL** with the MMCE/SMB argv autolaunch, from higorhgon/Open-PS2-Loader's manual releases (the
//!   OFFICIALPINNED build, or the RetroAchievements one): `APPS/OPL/RIPTOPL.ELF`
//! - **Neutrino**, from rickgaiser/neutrino's 7z: `APPS/neutrino/` (`neutrino.elf`, `modules/`, `config/`...)
//! - **Ember** (a beta, by Gageformer), from Gageformer/Ember's release: `EMBER/` with its licence, plus the
//!   `bios.bin` the user gives (Ember doesn't come with one, and it can't be downloaded)
//!
//! Everything is downloaded first; then the files to write are shown, and only written after confirming.

use crate::config;
use crate::memcard::Card;
use serde_json::Value;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const OSDHUB_REPO: &str = "higorhgon/osdmenu";
pub const OPL_REPO: &str = "higorhgon/Open-PS2-Loader";
pub const NEUTRINO_REPO: &str = "rickgaiser/neutrino";
pub const EMBER_REPO: &str = "Gageformer/Ember";

/// Where OSDHub is installed in the memory card, which the boot loader starts
pub const BOOT_ELF: &str = "BOOT/BOOT.ELF";
pub const OPL_DIR: &str = "APPS/OPL";
pub const NEUTRINO_DIR: &str = "APPS/neutrino";
pub const EMBER_DIR: &str = "EMBER";

/// Where OSDHub goes
#[derive(Clone, PartialEq, Debug)]
pub enum Target {
    /// A memory card image, like the one an MMCE device boots from
    Card(PathBuf),
    /// A folder holding BOOT/ and SYS-CONF/, to copy to a memory card
    Folder(PathBuf),
}

impl Default for Target {
    fn default() -> Target {
        Target::Folder(PathBuf::new())
    }
}

impl Target {
    pub fn path(&self) -> &Path {
        match self {
            Target::Card(path) | Target::Folder(path) => path,
        }
    }

    /// Whether `file` (like `SYS-CONF/OSDMENU.CNF`) is there already
    fn has(&self, file: &str) -> Result<bool, String> {
        match self {
            Target::Card(card) => {
                let image = fs::read(card).map_err(|e| format!("{}: {e}", card.display()))?;
                Ok(Card::open(&image)?.exists(file))
            }
            Target::Folder(folder) => Ok(on_device(folder, file).exists()),
        }
    }
}

/// The folder for a memory card, on the device, when there's no memory card image
pub const FOLDER: &str = "OSDHUB-MC";

/// What to install besides OSDHub
#[derive(Clone, Default, Debug)]
pub struct Choice {
    pub opl: bool,
    /// The RetroAchievements build of RiptOPL
    pub opl_ra: bool,
    pub neutrino: bool,
    pub ember: bool,
    /// The PS1 BIOS to copy to EMBER/bios.bin
    pub bios: Option<PathBuf>,
}

/// The files to write, downloaded
#[derive(Default, Debug)]
pub struct Plan {
    /// Where OSDHub goes
    pub target: Target,
    /// Files to write in the memory card
    pub card_files: Vec<(String, Vec<u8>)>,
    /// Files to write on the device, relative to its root
    pub files: Vec<(String, Vec<u8>)>,
    /// Folders to create on the device
    pub dirs: Vec<String>,
    /// What was downloaded, and from where
    pub sources: Vec<String>,
    /// Things to know, like a missing BIOS
    pub notes: Vec<String>,
    /// Whether the memory card (or folder) gets the example OSDMENU.CNF
    pub creates_cnf: bool,
}

/// A release asset
struct Asset {
    name: String,
    url: String,
}

struct Release {
    tag: String,
    prerelease: bool,
    assets: Vec<Asset>,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .user_agent(concat!("osdhub-manager/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn releases(agent: &ureq::Agent, repo: &str) -> Result<Vec<Release>, String> {
    // OSDHUB_GITHUB_API points to another server, to test the installer
    let api =
        std::env::var("OSDHUB_GITHUB_API").unwrap_or_else(|_| "https://api.github.com".to_string());
    let url = format!("{api}/repos/{repo}/releases?per_page=30");
    let body = agent
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("{repo}: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("{repo}: {e}"))?;
    let json: Value = serde_json::from_str(&body).map_err(|e| format!("{repo}: {e}"))?;
    let list = json
        .as_array()
        .ok_or(format!("{repo}: unexpected answer from GitHub"))?;
    Ok(list
        .iter()
        .filter(|r| !r["draft"].as_bool().unwrap_or(false))
        .map(|r| Release {
            tag: r["tag_name"].as_str().unwrap_or_default().to_string(),
            prerelease: r["prerelease"].as_bool().unwrap_or(false),
            assets: r["assets"]
                .as_array()
                .map(|assets| {
                    assets
                        .iter()
                        .map(|a| Asset {
                            name: a["name"].as_str().unwrap_or_default().to_string(),
                            url: a["browser_download_url"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect())
}

fn download(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    agent
        .get(url)
        .call()
        .map_err(|e| format!("{url}: {e}"))?
        .body_mut()
        .with_config()
        .limit(256 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("{url}: {e}"))
}

/// The files in a zip archive, as (path, contents)
pub fn unzip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().replace('\\', "/");
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).map_err(|e| e.to_string())?;
        files.push((name, contents));
    }
    Ok(files)
}

/// The files in a 7z archive, as (path, contents)
pub fn un7z(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive =
        sevenz_rust2::ArchiveReader::new(Cursor::new(bytes), sevenz_rust2::Password::empty())
            .map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    archive
        .for_each_entries(|entry, reader| {
            if !entry.is_directory() {
                let mut contents = Vec::new();
                reader.read_to_end(&mut contents)?;
                files.push((entry.name().replace('\\', "/"), contents));
            }
            Ok(true)
        })
        .map_err(|e| e.to_string())?;
    Ok(files)
}

/// The files of an archive under the folder holding `marker` (found without case), with paths relative to it
fn under_marker(files: Vec<(String, Vec<u8>)>, marker: &str) -> Option<Vec<(String, Vec<u8>)>> {
    let found = files.iter().find(|(name, _)| {
        name.rsplit('/')
            .next()
            .is_some_and(|n| n.eq_ignore_ascii_case(marker))
    })?;
    let base = found.0[..found.0.len() - marker.len()].to_string();
    Some(
        files
            .into_iter()
            .filter_map(|(name, contents)| {
                name.strip_prefix(&base)
                    .map(|rest| (rest.to_string(), contents))
            })
            .filter(|(name, _)| !name.is_empty())
            .collect(),
    )
}

/// A file of OSDHub's package by its name: at the root of the unsigned package, or in `examples/` (the example
/// OSDMENU.CNF) of the full one, so the one nearest the root
fn package_file<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a (String, Vec<u8>)> {
    files
        .iter()
        .filter(|(path, _)| {
            path.rsplit('/')
                .next()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .min_by_key(|(path, _)| path.matches('/').count())
}

/// Paths on the device keep the case of the folders already there (FAT and exFAT ignore it, but Linux may not)
fn on_device(root: &Path, path: &str) -> PathBuf {
    let mut current = root.to_path_buf();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        let existing = fs::read_dir(&current).ok().and_then(|entries| {
            entries
                .filter_map(|e| e.ok())
                .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))
                .map(|e| e.path())
        });
        current = existing.unwrap_or_else(|| current.join(part));
    }
    current
}

/// Downloads what `choice` needs and plans where it goes, reporting each step with `progress`
pub fn prepare(
    root: &Path,
    target: &Target,
    choice: &Choice,
    progress: &dyn Fn(String),
) -> Result<Plan, String> {
    let agent = agent();
    let mut plan = Plan {
        target: target.clone(),
        ..Plan::default()
    };

    // OSDHub, into the memory card
    progress(format!(
        "Looking for OSDHub's latest release ({OSDHUB_REPO})..."
    ));
    let release = releases(&agent, OSDHUB_REPO)?
        .into_iter()
        .find(|r| {
            r.assets
                .iter()
                .any(|a| a.name.starts_with("osdmenu-") && a.name.ends_with(".zip"))
        })
        .ok_or(format!(
            "{OSDHUB_REPO} has no release with an osdmenu-*.zip package yet"
        ))?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name.starts_with("osdmenu-") && a.name.ends_with(".zip"))
        .unwrap();
    progress(format!("Downloading {}...", asset.name));
    let files = unzip(&download(&agent, &asset.url)?)?;
    let elf =
        package_file(&files, "osdmenu.elf").ok_or(format!("{} has no osdmenu.elf", asset.name))?;
    plan.card_files.push((BOOT_ELF.to_string(), elf.1.clone()));
    if !target.has(config::CNF_PATH)? {
        let example = package_file(&files, "OSDMENU.CNF")
            .ok_or(format!("{} has no example OSDMENU.CNF", asset.name))?;
        plan.card_files
            .push((config::CNF_PATH.to_string(), example.1.clone()));
        plan.creates_cnf = true;
    }
    // The icons of the folders in the PS2's Browser, from the release's sources: SYS-CONF's as KELFBinder installs
    // it, and OSDMenu's package icon for BOOT. A folder without one shows as corrupted data
    let icons: [(&str, &str, &[&str]); 2] = [
        (
            "SYS-CONF",
            "utils/res/kelfbinder/ASSETS/SYS-CONF",
            &["icon.sys", "list.icn"],
        ),
        (
            "BOOT",
            "utils/res/psu",
            &["icon.sys", "list.icn", "copy.icn", "del.icn"],
        ),
    ];
    // OSDHUB_GITHUB_RAW points to another server, to test the installer
    let raw = std::env::var("OSDHUB_GITHUB_RAW")
        .unwrap_or_else(|_| "https://raw.githubusercontent.com".to_string());
    for (folder, source, files) in icons {
        if target.has(&format!("{folder}/icon.sys"))? {
            continue;
        }
        // All of them or none: an icon.sys without its icons is worse than none
        let downloaded: Result<Vec<(String, Vec<u8>)>, String> = files
            .iter()
            .map(|file| {
                let url = format!("{raw}/{OSDHUB_REPO}/{}/{source}/{file}", release.tag);
                download(&agent, &url).map(|contents| (format!("{folder}/{file}"), contents))
            })
            .collect();
        match downloaded {
            Ok(icons) => plan.card_files.extend(icons),
            Err(e) => plan.notes.push(format!(
                "{folder}'s icon wasn't downloaded, so the Browser shows it as corrupted data ({e})"
            )),
        }
    }
    plan.sources
        .push(format!("OSDHub {} ({OSDHUB_REPO})", release.tag));

    if choice.opl {
        progress(format!(
            "Looking for RiptOPL's latest release with the argv autolaunch ({OPL_REPO})..."
        ));
        let suffix = if choice.opl_ra {
            "-PS2DEVPINNED-RA.zip"
        } else {
            "-OFFICIALPINNED.zip"
        };
        // The manual releases: "rolling" and the v* tags are built from rebuild/main, without the autolaunch
        let release = releases(&agent, OPL_REPO)?
            .into_iter()
            .filter(|r| r.tag != "rolling" && !r.tag.starts_with('v'))
            .find(|r| r.assets.iter().any(|a| a.name.ends_with(suffix)))
            .ok_or(format!(
                "{OPL_REPO} has no manual release with a *{suffix} build yet"
            ))?;
        let asset = release
            .assets
            .iter()
            .find(|a| a.name.ends_with(suffix))
            .unwrap();
        progress(format!("Downloading {}...", asset.name));
        let files = unzip(&download(&agent, &asset.url)?)?;
        let elf = files
            .iter()
            .find(|(name, _)| name.to_ascii_uppercase().ends_with(".ELF"))
            .ok_or(format!("{} has no ELF", asset.name))?;
        plan.files
            .push((format!("{OPL_DIR}/RIPTOPL.ELF"), elf.1.clone()));
        for (name, contents) in &files {
            if name.eq_ignore_ascii_case("LICENSE.txt") {
                plan.files
                    .push((format!("{OPL_DIR}/LICENSE.txt"), contents.clone()));
            }
        }
        plan.sources.push(format!(
            "RiptOPL {}{} ({OPL_REPO})",
            release.tag,
            if choice.opl_ra {
                " RetroAchievements"
            } else {
                ""
            }
        ));
    }

    if choice.neutrino {
        progress(format!(
            "Looking for Neutrino's latest release ({NEUTRINO_REPO})..."
        ));
        let release = releases(&agent, NEUTRINO_REPO)?
            .into_iter()
            .find(|r| !r.prerelease && r.assets.iter().any(|a| a.name.ends_with(".7z")))
            .ok_or(format!("{NEUTRINO_REPO} has no release with a .7z package"))?;
        let asset = release
            .assets
            .iter()
            .find(|a| a.name.ends_with(".7z"))
            .unwrap();
        progress(format!("Downloading {}...", asset.name));
        let files = under_marker(un7z(&download(&agent, &asset.url)?)?, "neutrino.elf")
            .ok_or(format!("{} has no neutrino.elf", asset.name))?;
        plan.files.extend(
            files
                .into_iter()
                .map(|(name, contents)| (format!("{NEUTRINO_DIR}/{name}"), contents)),
        );
        plan.sources
            .push(format!("Neutrino {} ({NEUTRINO_REPO})", release.tag));
    }

    if choice.ember {
        progress(format!(
            "Looking for Ember's latest release ({EMBER_REPO})..."
        ));
        let release = releases(&agent, EMBER_REPO)?
            .into_iter()
            .find(|r| {
                r.assets.iter().any(|a| {
                    a.name.to_lowercase().ends_with(".zip")
                        || a.name.to_lowercase().ends_with(".elf")
                })
            })
            .ok_or(format!("{EMBER_REPO} has no release with Ember"))?;
        let mut ember = None;
        // A package with ember.elf, or ember.elf and the licence on their own
        for asset in release
            .assets
            .iter()
            .filter(|a| a.name.to_lowercase().ends_with(".zip"))
        {
            progress(format!("Downloading {}...", asset.name));
            if let Some(files) = under_marker(unzip(&download(&agent, &asset.url)?)?, "ember.elf") {
                ember = Some(files);
                break;
            }
        }
        if ember.is_none() {
            let mut files = Vec::new();
            for asset in &release.assets {
                let lower = asset.name.to_lowercase();
                if lower == "ember.elf"
                    || lower.starts_with("license")
                    || lower.starts_with("licence")
                {
                    progress(format!("Downloading {}...", asset.name));
                    files.push((asset.name.clone(), download(&agent, &asset.url)?));
                }
            }
            if files
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("ember.elf"))
            {
                ember = Some(files);
            }
        }
        let files = ember.ok_or(format!("Ember {} has no ember.elf", release.tag))?;
        for (name, contents) in files {
            let lower = name.to_lowercase();
            // Never a BIOS, and the user's own settings and games stay
            if lower.ends_with("bios.bin") || lower.starts_with("games/") {
                continue;
            }
            if lower == "settings.txt"
                && on_device(root, &format!("{EMBER_DIR}/settings.txt")).exists()
            {
                continue;
            }
            plan.files.push((format!("{EMBER_DIR}/{name}"), contents));
        }
        plan.dirs.push(format!("{EMBER_DIR}/games"));
        match &choice.bios {
            Some(bios) => {
                let contents = fs::read(bios).map_err(|e| format!("{}: {e}", bios.display()))?;
                plan.files.push((format!("{EMBER_DIR}/bios.bin"), contents));
            }
            None if !on_device(root, &format!("{EMBER_DIR}/bios.bin")).exists() => plan.notes.push(
                "Ember needs a PS1 BIOS dumped from your console as EMBER/bios.bin; it isn't downloaded".to_string(),
            ),
            None => {}
        }
        plan.sources.push(format!(
            "Ember {} by Gageformer (https://github.com/{EMBER_REPO}/releases), under its beta licence",
            release.tag
        ));
    }
    Ok(plan)
}

impl Plan {
    /// The files to write, as (where, whether it replaces a file), for the confirmation
    pub fn describe(&self, root: &Path) -> Vec<(String, bool)> {
        let mut lines: Vec<(String, bool)> = self
            .card_files
            .iter()
            .map(|(file, _)| {
                let replaces = self.target.has(file).unwrap_or(false);
                let place = match &self.target {
                    Target::Card(card) => format!("mc0:/{file} (in {})", card.display()),
                    Target::Folder(folder) => on_device(folder, file).display().to_string(),
                };
                (place, replaces)
            })
            .collect();
        lines.extend(
            self.files
                .iter()
                .map(|(file, _)| (file.clone(), on_device(root, file).exists())),
        );
        lines.extend(
            self.dirs
                .iter()
                .filter(|d| !on_device(root, d).exists())
                .map(|d| (format!("{d}/"), false)),
        );
        lines
    }

    /// Writes the files: the memory card through a copy that's checked, then the device's files
    pub fn apply(&self, root: &Path) -> Result<Vec<String>, String> {
        let mut log = Vec::new();
        let files: Vec<(&str, &[u8])> = self
            .card_files
            .iter()
            .map(|(f, c)| (f.as_str(), c.as_slice()))
            .collect();
        match &self.target {
            Target::Card(card) => {
                let backup = config::write_card(card, &files)?;
                log.push(format!(
                    "Installed OSDHub in {} as mc0:/{BOOT_ELF}; the previous memory card is in {}",
                    card.display(),
                    backup.display()
                ));
            }
            Target::Folder(folder) => {
                for (file, contents) in files {
                    let path = on_device(folder, file);
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|e| format!("{}: {e}", parent.display()))?;
                    }
                    fs::write(&path, contents).map_err(|e| format!("{}: {e}", path.display()))?;
                }
                log.push(format!(
                    "OSDHub is in {}: copy its BOOT and SYS-CONF folders to mc0:/ (with wLaunchELF, for example); \
                     BOOT.ELF replaces the one there, and skip SYS-CONF to keep the OSDMENU.CNF the memory card has",
                    folder.display()
                ));
            }
        }
        for (file, contents) in &self.files {
            let path = on_device(root, file);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            fs::write(&path, contents).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        for dir in &self.dirs {
            let path = on_device(root, dir);
            fs::create_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        if !self.files.is_empty() {
            log.push(format!(
                "{} file(s) written on {}",
                self.files.len(),
                root.display()
            ));
        }
        log.extend(self.notes.iter().cloned());
        Ok(log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memcard::tests::card_image;
    use std::io::Write;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        for (name, contents) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap();
        out.into_inner()
    }

    #[test]
    fn packages() {
        let files = vec![
            (
                "patcher/kelfbinder/ASSETS/SYS-CONF/OSDMENU.CNF".to_string(),
                b"deep".to_vec(),
            ),
            ("examples/OSDMENU.CNF".to_string(), b"example".to_vec()),
            ("osdmenu.elf".to_string(), b"elf".to_vec()),
        ];
        assert_eq!(package_file(&files, "OSDMENU.CNF").unwrap().1, b"example");
        assert_eq!(package_file(&files, "osdmenu.elf").unwrap().1, b"elf");
        assert!(package_file(&files, "hosdmenu.elf").is_none());
    }

    #[test]
    fn archives() {
        let zip = zip_of(&[
            ("pkg/ember.elf", b"elf"),
            ("pkg/LICENSE-BETA.txt", b"licence"),
            ("other.txt", b"x"),
        ]);
        let files = under_marker(unzip(&zip).unwrap(), "ember.elf").unwrap();
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["ember.elf", "LICENSE-BETA.txt"]);
        assert!(under_marker(unzip(&zip).unwrap(), "neutrino.elf").is_none());
    }

    #[test]
    fn device_paths() {
        let root =
            std::env::temp_dir().join(format!("osdhub-manager-install-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("apps/opl")).unwrap();
        // An existing folder keeps its case, the rest is created as given
        assert_eq!(
            on_device(&root, "APPS/OPL/RIPTOPL.ELF"),
            root.join("apps/opl/RIPTOPL.ELF")
        );
        assert_eq!(on_device(&root, "EMBER/games"), root.join("EMBER/games"));

        let card = root.join("BootCard.mcd");
        fs::write(&card, card_image(b"OSDSYS_menu_x = 400\n", false)).unwrap();
        let plan = Plan {
            target: Target::Card(card.clone()),
            card_files: vec![(BOOT_ELF.to_string(), vec![1; 3000])],
            files: vec![("APPS/OPL/RIPTOPL.ELF".to_string(), b"opl".to_vec())],
            dirs: vec!["EMBER/games".to_string()],
            ..Plan::default()
        };
        let lines = plan.describe(&root);
        assert_eq!(lines.len(), 3);
        assert!(!lines[0].1 && !lines[1].1);
        plan.apply(&root).unwrap();
        let written = Card::open(&fs::read(&card).unwrap()).unwrap();
        assert_eq!(written.read(BOOT_ELF).unwrap(), vec![1; 3000]);
        assert_eq!(
            written.read(config::CNF_PATH).unwrap(),
            b"OSDSYS_menu_x = 400\n"
        );
        assert_eq!(fs::read(root.join("apps/opl/RIPTOPL.ELF")).unwrap(), b"opl");
        assert!(root.join("EMBER/games").is_dir());
        // Installing again replaces them
        assert!(plan.describe(&root)[0].1 && plan.describe(&root)[1].1);

        // Or into a folder to copy to a memory card
        let folder = root.join(FOLDER);
        let plan = Plan {
            target: Target::Folder(folder.clone()),
            card_files: vec![
                (BOOT_ELF.to_string(), vec![2; 10]),
                (config::CNF_PATH.to_string(), b"x = 1\n".to_vec()),
            ],
            ..Plan::default()
        };
        let lines = plan.describe(&root);
        assert_eq!(
            lines[0],
            (
                folder.join("BOOT").join("BOOT.ELF").display().to_string(),
                false
            )
        );
        let log = plan.apply(&root).unwrap();
        assert!(log[0].contains("copy its BOOT and SYS-CONF folders"));
        assert_eq!(fs::read(folder.join("BOOT/BOOT.ELF")).unwrap(), vec![2; 10]);
        assert_eq!(
            fs::read(folder.join("SYS-CONF/OSDMENU.CNF")).unwrap(),
            b"x = 1\n"
        );
        assert!(plan.target.has(config::CNF_PATH).unwrap());
        fs::remove_dir_all(&root).unwrap();
    }
}

#[cfg(test)]
mod external {
    /// Lists a 7z archive given in OSDHUB_7Z: `OSDHUB_7Z=x.7z cargo test external_7z -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn external_7z() {
        let bytes = std::fs::read(std::env::var("OSDHUB_7Z").unwrap()).unwrap();
        let files = super::under_marker(super::un7z(&bytes).unwrap(), "neutrino.elf").unwrap();
        for (name, contents) in &files {
            println!("{name} {}", contents.len());
        }
    }
}
