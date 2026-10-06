//! Downloads OPL ART images (`ART/<title ID>_COV.jpg`, `_ICO.png`...) for the games found on a device.
//!
//! Sources, tried in order:
//! - OPL Manager's art database, from a fork of its dump on GitHub (`PS1/<ID>/<ID>_COV.png`, `PS2/...`),
//!   with every OPL art type, including the disc (`ICO`); its backup on archive.org has the same layout
//! - xlenore's PS1 and PS2 cover collections on GitHub (`covers/default/SLUS-20212.jpg`), case covers only

use crate::games::{Console, Game};
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

/// The dump of OPL Manager's art database on GitHub (a fork of Luden02/psx-ps2-opl-art-database), whose images are PNG.
/// Its backup on archive.org works too, as the files inside its zip are downloaded one by one:
/// https://archive.org/download/OPLM_ART_2024_09/OPLM_ART_2024_09.zip
pub const DEFAULT_OPLM_URL: &str =
    "https://raw.githubusercontent.com/higorhgon/psx-ps2-opl-art-database/main";
const XLENORE_PS1_URL: &str =
    "https://raw.githubusercontent.com/xlenore/psx-covers/main/covers/default";
const XLENORE_PS2_URL: &str =
    "https://raw.githubusercontent.com/xlenore/ps2-covers/main/covers/default";
/// Tried in this order: OPL Manager's images are mostly PNG
const EXTENSIONS: [&str; 2] = ["png", "jpg"];

/// OPL art types: the case cover and the disc are the ones OSDHub shows
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArtType {
    Cov,
    Ico,
}

impl ArtType {
    pub fn parse(s: &str) -> Option<ArtType> {
        match s.to_ascii_uppercase().as_str() {
            "COV" => Some(ArtType::Cov),
            "ICO" => Some(ArtType::Ico),
            _ => None,
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            ArtType::Cov => "COV",
            ArtType::Ico => "ICO",
        }
    }
}

#[derive(Clone)]
pub struct Sources {
    /// OPL Manager's art database, None to skip it
    pub oplm_url: Option<String>,
    pub xlenore: bool,
}

/// What happened to one image
pub enum Outcome {
    Exists(String),
    Downloaded { file: String, source: &'static str },
    WouldDownload,
    NotFound,
    Failed(String),
}

/// A downloaded image: its data, extension and source
type Image = (Vec<u8>, &'static str, &'static str);

pub struct Downloader {
    agent: ureq::Agent,
    sources: Sources,
}

impl Downloader {
    pub fn new(sources: Sources) -> Downloader {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(concat!("osdhub-manager/", env!("CARGO_PKG_VERSION")))
            .build();
        Downloader {
            agent: config.into(),
            sources,
        }
    }

    /// Downloads `url`: Ok(None) when it doesn't exist
    fn get(&self, url: &str) -> Result<Option<Vec<u8>>, String> {
        match self.agent.get(url).call() {
            Ok(mut response) => response
                .body_mut()
                .with_config()
                .limit(16 * 1024 * 1024)
                .read_to_vec()
                .map(Some)
                .map_err(|e| e.to_string()),
            Err(ureq::Error::StatusCode(404)) | Err(ureq::Error::StatusCode(403)) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    /// The image of `art` for the game with title ID `id`, as (data, extension, source)
    fn fetch(&self, console: Console, id: &str, art: ArtType) -> Result<Option<Image>, String> {
        let mut last_error = None;
        if let Some(base) = &self.sources.oplm_url {
            for ext in EXTENSIONS {
                let url = format!(
                    "{}/{}/{id}/{id}_{}.{ext}",
                    base.trim_end_matches('/'),
                    console.art_folder(),
                    art.suffix()
                );
                match self.get(&url) {
                    Ok(Some(data)) => return Ok(Some((data, ext, "OPL Manager"))),
                    Ok(None) => {}
                    Err(e) => last_error = Some(e),
                }
            }
        }
        if self.sources.xlenore && art == ArtType::Cov {
            // SLUS_202.12 is SLUS-20212 there
            let serial = format!("{}-{}{}", &id[..4], &id[5..8], &id[9..11]);
            let base = if console == Console::Ps1 {
                XLENORE_PS1_URL
            } else {
                XLENORE_PS2_URL
            };
            match self.get(&format!("{base}/{serial}.jpg")) {
                Ok(Some(data)) => return Ok(Some((data, "jpg", "xlenore"))),
                Ok(None) => {}
                Err(e) => last_error = Some(e),
            }
        }
        match last_error {
            Some(e) => Err(e),
            None => Ok(None),
        }
    }

    /// Downloads the `art` image of a game into `art_dir` as `<ID>_<TYPE>.<ext>`,
    /// unless an image of that type is already there (in any format) and `force` isn't set
    pub fn download(
        &self,
        game: &Game,
        id: &str,
        art: ArtType,
        art_dir: &Path,
        force: bool,
        dry_run: bool,
    ) -> Outcome {
        if !force && let Some(existing) = existing_art(art_dir, id, art) {
            return Outcome::Exists(existing);
        }
        if dry_run {
            return Outcome::WouldDownload;
        }
        match self.fetch(game.console, id, art) {
            Ok(Some((data, ext, source))) => {
                let file = format!("{id}_{}.{ext}", art.suffix());
                let write = fs::create_dir_all(art_dir).and_then(|_| {
                    // Replace the other format too, so only the new image is used
                    if force {
                        remove_art(art_dir, id, art)?;
                    }
                    fs::write(art_dir.join(&file), &data)
                });
                match write {
                    Ok(()) => Outcome::Downloaded { file, source },
                    Err(e) => Outcome::Failed(e.to_string()),
                }
            }
            Ok(None) => Outcome::NotFound,
            Err(e) => Outcome::Failed(e),
        }
    }
}

/// The name of an existing `<ID>_<TYPE>.jpg/png` image in `art_dir`, matching without case
pub fn existing_art(art_dir: &Path, id: &str, art: ArtType) -> Option<String> {
    let wanted: Vec<String> = EXTENSIONS
        .iter()
        .map(|ext| format!("{id}_{}.{ext}", art.suffix()).to_lowercase())
        .collect();
    fs::read_dir(art_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|name| wanted.contains(&name.to_lowercase()))
}

fn remove_art(art_dir: &Path, id: &str, art: ArtType) -> io::Result<()> {
    while let Some(existing) = existing_art(art_dir, id, art) {
        fs::remove_file(art_dir.join(existing))?;
    }
    Ok(())
}
