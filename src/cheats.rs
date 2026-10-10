//! Widescreen cheats for OPL from PS2-Widescreen/OPL-Widescreen-Cheats: one `CHT/<title ID>.cht` file per PS2 game,
//! which OPL (and RiptOPL) loads when its PS2RD cheat engine is on, in "Auto-select cheats" mode.
//! The files are downloaded one by one from the repository and written to `CHT/` at the device root,
//! with CRLF line endings like the repository's own package.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The repository's files: `CHT/<ID>.cht` (a few with the extension in capitals)
pub const DEFAULT_URL: &str =
    "https://raw.githubusercontent.com/PS2-Widescreen/OPL-Widescreen-Cheats/main/CHT";
const EXTENSIONS: [&str; 2] = ["cht", "CHT"];

/// The folder OPL reads the cheats from, at the device root
pub fn cheat_dir(root: &Path) -> PathBuf {
    root.join("CHT")
}

/// The existing cheat file of the game with title ID `id` in `dir`, matching without case
pub fn existing(dir: &Path, id: &str) -> Option<PathBuf> {
    let wanted = format!("{id}.cht").to_lowercase();
    fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().to_lowercase() == wanted)
        .map(|e| e.path())
}

/// What happened to one game
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Written, replacing an existing file when it's set
    Written {
        file: String,
        replaced: bool,
    },
    NotAvailable,
    Failed(String),
}

pub struct Downloader {
    agent: ureq::Agent,
    url: String,
}

impl Downloader {
    /// Downloads from the repository, or from `OSDHUB_CHEATS_URL` (another server, to test it)
    pub fn new() -> Downloader {
        Downloader::with_url(
            &std::env::var("OSDHUB_CHEATS_URL").unwrap_or_else(|_| DEFAULT_URL.to_string()),
        )
    }

    pub fn with_url(url: &str) -> Downloader {
        Downloader {
            agent: crate::net::agent(Duration::from_secs(60)),
            url: url.to_string(),
        }
    }

    /// The cheat file of the game with title ID `id`: Ok(None) when the repository has none
    fn fetch(&self, id: &str) -> Result<Option<Vec<u8>>, String> {
        for ext in EXTENSIONS {
            let url = format!("{}/{id}.{ext}", self.url.trim_end_matches('/'));
            match self.agent.get(&url).call() {
                Ok(mut response) => {
                    return response
                        .body_mut()
                        .with_config()
                        .limit(1024 * 1024)
                        .read_to_vec()
                        .map(Some)
                        .map_err(|e| e.to_string());
                }
                Err(ureq::Error::StatusCode(404)) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(None)
    }

    /// Downloads the cheat of the game with title ID `id` into `dir`, replacing an existing one
    pub fn install(&self, dir: &Path, id: &str) -> Outcome {
        match self.fetch(id) {
            Ok(Some(data)) => match write(dir, id, &data) {
                Ok((file, replaced)) => Outcome::Written { file, replaced },
                Err(e) => Outcome::Failed(e),
            },
            Ok(None) => Outcome::NotAvailable,
            Err(e) => Outcome::Failed(e),
        }
    }
}

/// Writes a cheat file as `<ID>.cht`, over the existing file of the game (whatever its case),
/// returning its name and whether it replaced one
pub fn write(dir: &Path, id: &str, data: &[u8]) -> Result<(String, bool), String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let existing = existing(dir, id);
    let path = existing
        .clone()
        .unwrap_or_else(|| dir.join(format!("{id}.cht")));
    fs::write(&path, crlf(data)).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok((file, existing.is_some()))
}

/// The text with CRLF line endings
fn crlf(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 16);
    for (i, &byte) in data.iter().enumerate() {
        if byte == b'\n' && (i == 0 || data[i - 1] != b'\r') {
            out.push(b'\r');
        }
        out.push(byte);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "osdhub-manager-cheats-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// Serves `files` (path, body) over HTTP on localhost, 404 for anything else, returning the base URL
    fn serve(files: Vec<(&'static str, &'static str)>) -> String {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(&stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap_or_default();
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 2 {
                    line.clear();
                }
                let path = request.split(' ').nth(1).unwrap_or_default().to_string();
                let response = match files.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                    None => {
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_string()
                    }
                };
                let _ = (&stream).write_all(response.as_bytes());
            }
        });
        url
    }

    #[test]
    fn downloads() {
        let url = serve(vec![
            ("/CHT/SLUS_203.12.cht", "\"Final Fantasy X\"\nMastercode\n"),
            ("/CHT/SLUS_205.17.CHT", "\"Capitals\"\n"),
        ]);
        let downloader = Downloader::with_url(&format!("{url}/CHT/"));
        let dir = temp_dir("download");
        assert_eq!(
            downloader.install(&dir, "SLUS_203.12"),
            Outcome::Written {
                file: "SLUS_203.12.cht".into(),
                replaced: false
            }
        );
        assert_eq!(
            fs::read(dir.join("SLUS_203.12.cht")).unwrap(),
            b"\"Final Fantasy X\"\r\nMastercode\r\n"
        );
        // The repository has a few files with the extension in capitals
        assert_eq!(
            downloader.install(&dir, "SLUS_205.17"),
            Outcome::Written {
                file: "SLUS_205.17.cht".into(),
                replaced: false
            }
        );
        assert_eq!(
            downloader.install(&dir, "SLUS_999.99"),
            Outcome::NotAvailable
        );
        assert_eq!(
            downloader.install(&dir, "SLUS_203.12"),
            Outcome::Written {
                file: "SLUS_203.12.cht".into(),
                replaced: true
            }
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Downloads from the repository and compares with its checkout in OSDHUB_CHEATS_CHECKOUT
    /// (`cargo test cheats::tests::repository -- --ignored`)
    #[test]
    #[ignore]
    fn repository() {
        let checkout = PathBuf::from(std::env::var("OSDHUB_CHEATS_CHECKOUT").unwrap()).join("CHT");
        let downloader = Downloader::with_url(DEFAULT_URL);
        let dir = temp_dir("repository");
        for (id, file) in [
            ("SLUS_203.12", "SLUS_203.12.cht"),
            ("SLUS_205.17", "SLUS_205.17.CHT"),
        ] {
            assert!(matches!(
                downloader.install(&dir, id),
                Outcome::Written { .. }
            ));
            assert_eq!(
                fs::read(dir.join(format!("{id}.cht"))).unwrap(),
                fs::read(checkout.join(file)).unwrap(),
                "{id}"
            );
        }
        assert_eq!(
            downloader.install(&dir, "SLUS_999.99"),
            Outcome::NotAvailable
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn line_endings() {
        assert_eq!(crlf(b"a\nb\r\nc\n"), b"a\r\nb\r\nc\r\n");
        assert_eq!(crlf(b"\n"), b"\r\n");
        assert_eq!(crlf(b"no newline"), b"no newline");
    }

    #[test]
    fn writes_and_replaces() {
        let dir = temp_dir("write");
        assert_eq!(existing(&dir, "SLUS_203.12"), None);
        let (file, replaced) = write(&dir, "SLUS_203.12", b"\"FFX\"\nMastercode\n").unwrap();
        assert_eq!((file.as_str(), replaced), ("SLUS_203.12.cht", false));
        assert_eq!(
            fs::read(dir.join("SLUS_203.12.cht")).unwrap(),
            b"\"FFX\"\r\nMastercode\r\n"
        );

        // An existing file is found and replaced whatever the case of its name
        fs::remove_file(dir.join("SLUS_203.12.cht")).unwrap();
        fs::write(dir.join("slus_203.12.CHT"), b"old").unwrap();
        assert_eq!(
            existing(&dir, "SLUS_203.12"),
            Some(dir.join("slus_203.12.CHT"))
        );
        let (file, replaced) = write(&dir, "SLUS_203.12", b"new\n").unwrap();
        assert_eq!((file.as_str(), replaced), ("slus_203.12.CHT", true));
        assert_eq!(fs::read(dir.join("slus_203.12.CHT")).unwrap(), b"new\r\n");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
