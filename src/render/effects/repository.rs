//! Plugin repositories: a JSON list of plugins on the web, each with its versions and,
//! per version, a file to download for each system. The plugin manager lists the plugins
//! of every repository and installs one in the user's plugins folder.
//!
//! The list (format 1):
//!
//! ```json
//! {"format": 1, "name": "...", "plugins": [{
//!     "id": "halftone", "name": "Halftone", "description": "...", "author": "...",
//!     "homepage": "https://...", "license": "MIT", "kind": "library" | "python",
//!     "latest": "0.1.0",
//!     "versions": [{"version": "0.1.0", "abi": 2, "min_app": "0.16.0",
//!         "compatible_from": "0.1.0",
//!         "effects": ["Halftone"], "generators": ["Dot grid"],
//!         "files": {"linux-x86_64": {"url": "https://...", "sha256": "...", "size": 1234}}}]
//! }]}
//! ```
//!
//! A library has one file per system (`linux-x86_64`, `windows-x86_64`...), a Python
//! plugin one for all (`any`). `latest` is the newest version released (none later is
//! offered); a version's `compatible_from`, the oldest version whose saved settings it
//! still reads (all of them if absent). Other fields are ignored.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use artwork_plugin_sdk::abi;
use serde::Deserialize;

/// The repository of the app's own plugins, always listed.
pub const OFFICIAL: &str = "https://raw.githubusercontent.com/bzhmaddog/artwork-creator-plugins/refs/heads/main/plugins.json";

/// The format of the list this app reads.
const FORMAT: u32 = 1;
/// Longest list read.
const MAX_LIST: u64 = 4 << 20;

/// A repository's list of plugins.
#[derive(Clone, Debug, Deserialize)]
pub struct Repository {
    pub format: u32,
    pub name: String,
    pub plugins: Vec<Entry>,
}

/// A plugin of a repository.
#[derive(Clone, Debug, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    pub kind: Kind,
    /// The newest version released: none later is offered.
    #[serde(default)]
    pub latest: Option<String>,
    pub versions: Vec<Version>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Library,
    Python,
}

/// A version of a plugin.
#[derive(Clone, Debug, Deserialize)]
pub struct Version {
    pub version: String,
    /// The plugin interface version of a library.
    #[serde(default)]
    pub abi: Option<u32>,
    /// The oldest version of the app it works with.
    #[serde(default)]
    pub min_app: Option<String>,
    /// The oldest version of the plugin whose saved settings it still reads; all of them
    /// if `None`.
    #[serde(default)]
    pub compatible_from: Option<String>,
    /// What changed in it.
    #[serde(default)]
    pub changes: String,
    /// The names of the effects it adds.
    #[serde(default)]
    pub effects: Vec<String>,
    /// The names of the generators it adds.
    #[serde(default)]
    pub generators: Vec<String>,
    /// By system (`linux-x86_64`...), `any` for a Python plugin.
    pub files: BTreeMap<String, Download>,
}

/// A file to download.
#[derive(Clone, Debug, Deserialize, Eq)]
pub struct Download {
    pub url: String,
    /// In hex.
    pub sha256: String,
    /// In bytes.
    pub size: u64,
}

impl Download {
    /// The name of the file, the last part of its URL.
    pub fn file_name(&self) -> &str {
        let path = self.url.split(['?', '#']).next().unwrap_or_default();
        path.rsplit('/').next().unwrap_or_default()
    }
}

/// The key of this system's files: `linux-x86_64`, `windows-x86_64`...
pub fn system() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// A version number's parts (`1.2.3` -> [1, 2, 3]), a pre-release suffix (`-beta`) left
/// out; `None` if it isn't one.
pub fn version_parts(v: &str) -> Option<Vec<u64>> {
    let v = v.split(['-', '+']).next()?;
    v.split('.').map(|p| p.parse().ok()).collect()
}

impl Entry {
    /// The latest version that works with this app on this system, and its file.
    pub fn best(&self) -> Option<(&Version, &Download)> {
        self.best_for(env!("CARGO_PKG_VERSION"), &system())
    }

    fn best_for(&self, app: &str, system: &str) -> Option<(&Version, &Download)> {
        let app = version_parts(app)?;
        let latest = self.latest.as_deref().and_then(version_parts);
        let key = match self.kind {
            Kind::Library => system,
            Kind::Python => "any",
        };
        self.versions
            .iter()
            .filter(|v| {
                let min = v.min_app.as_deref().and_then(version_parts);
                let abi = v.abi.unwrap_or(abi::OLDEST_ABI_VERSION);
                min.is_none_or(|m| m <= app)
                    && latest
                        .as_ref()
                        .is_none_or(|l| version_parts(&v.version).as_ref() <= Some(l))
                    && (self.kind == Kind::Python
                        || (abi::OLDEST_ABI_VERSION..=abi::ABI_VERSION).contains(&abi))
            })
            .filter_map(|v| v.files.get(key).map(|d| (v, d)))
            .max_by_key(|(v, _)| version_parts(&v.version))
    }
}

/// The lists of some repositories: each address with its list, or why it could not be
/// read.
pub type Catalog = Vec<(String, Result<Repository, String>)>;

/// Downloads the lists at `urls`, all at once.
pub fn fetch_all(urls: &[String]) -> Catalog {
    std::thread::scope(|s| {
        let handles: Vec<_> = urls.iter().map(|u| s.spawn(|| fetch(u))).collect();
        let results = handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err("stopped".into())));
        urls.iter().cloned().zip(results).collect()
    })
}

/// What a repository says about a plugin file installed, found by its content.
#[derive(Clone, Debug, PartialEq)]
pub struct Known {
    pub description: String,
    /// The version of the file installed.
    pub version: String,
    /// The names of the effects and generators of that version.
    pub effects: Vec<String>,
    pub generators: Vec<String>,
    /// A later version that works here: its number, what changed in it, and its file.
    pub update: Option<Update>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    /// The plugin's id in the repository.
    pub id: String,
    pub version: String,
    pub changes: String,
    pub kind: Kind,
    pub download: Download,
    /// Its `compatible_from`.
    pub compatible_from: Option<String>,
    /// It can't read the settings the installed version saved (its `compatible_from` is
    /// later).
    pub breaking: bool,
}

impl PartialEq for Download {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url && self.sha256.eq_ignore_ascii_case(&other.sha256)
    }
}

/// What `repos` (address and list) say about the plugin files of `hashes` (path and
/// SHA-256), those one of them lists (the first that does): for a file installed from a
/// repository, only that one (see [`plugin::origin`](super::plugin::origin)), found by
/// its content or else by the id and version recorded when it was installed (a version
/// the list no longer has). Any other file whose content none lists (changed by hand,
/// from elsewhere) is left out: it is never offered an update.
pub fn known<'a>(
    hashes: &[(PathBuf, String)],
    repos: impl IntoIterator<Item = (&'a str, &'a Repository)> + Clone,
) -> BTreeMap<PathBuf, Known> {
    use super::plugin::{origin, origin_of, recorded};
    let mut found = BTreeMap::new();
    for (path, hash) in hashes {
        let from = origin_of(path);
        let plugins = || {
            (repos.clone().into_iter())
                .filter(|(url, _)| from.is_none_or(|o| o == origin(url)))
                .flat_map(|(_, r)| &r.plugins)
        };
        let by_content = plugins().find_map(|p| {
            let v = p.versions.iter().find(|v| {
                v.files
                    .values()
                    .any(|d| d.sha256.eq_ignore_ascii_case(hash))
            })?;
            Some((p, v.version.clone(), Some(v)))
        });
        let listed = by_content.or_else(|| {
            let entry = recorded(path)?.entry;
            let p = plugins().find(|p| p.id == entry.id)?;
            let v = p.versions.iter().find(|v| v.version == entry.version);
            Some((p, entry.version, v))
        });
        let Some((p, version, v)) = listed else {
            continue;
        };
        let installed = version_parts(&version);
        let update = p
            .best()
            .filter(|(best, _)| version_parts(&best.version) > installed)
            .map(|(best, d)| Update {
                id: p.id.clone(),
                version: best.version.clone(),
                changes: best.changes.clone(),
                kind: p.kind,
                download: d.clone(),
                compatible_from: best.compatible_from.clone(),
                breaking: (best.compatible_from.as_deref())
                    .is_some_and(|c| version_parts(c) > installed),
            });
        found.insert(
            path.clone(),
            Known {
                description: p.description.clone(),
                version,
                effects: v.map(|v| v.effects.clone()).unwrap_or_default(),
                generators: v.map(|v| v.generators.clone()).unwrap_or_default(),
                update,
            },
        );
    }
    found
}

/// The SHA-256 of each file of `paths` that can be read.
pub fn hashes<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Vec<(PathBuf, String)> {
    paths
        .into_iter()
        .filter_map(|p| Some((p.to_owned(), super::plugin::pending(p).ok()?.hash)))
        .collect()
}

/// Reads a repository's list, checking it: what is wrong with it, otherwise.
pub fn parse(json: &str) -> Result<Repository, String> {
    let repo: Repository = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if repo.format != FORMAT {
        return Err(format!(
            "format {} not supported (this app reads format {FORMAT})",
            repo.format
        ));
    }
    if repo.name.trim().is_empty() {
        return Err("the repository has no name".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for p in &repo.plugins {
        let what = format!("plugin \"{}\"", p.id);
        if p.id.trim().is_empty() || p.name.trim().is_empty() {
            return Err("a plugin has no id or no name".into());
        }
        if !ids.insert(&p.id) {
            return Err(format!("{what} is listed twice"));
        }
        if p.latest
            .as_deref()
            .is_some_and(|l| version_parts(l).is_none())
        {
            return Err(format!("{what}: latest is not a version number"));
        }
        if p.versions.is_empty() {
            return Err(format!("{what} has no versions"));
        }
        for v in &p.versions {
            let what = format!("{what} {}", v.version);
            let min = v.min_app.as_deref();
            let compatible = v.compatible_from.as_deref();
            if version_parts(&v.version).is_none()
                || min.is_some_and(|m| version_parts(m).is_none())
                || compatible.is_some_and(|c| version_parts(c).is_none())
            {
                return Err(format!("{what}: not a version number"));
            }
            if v.files.is_empty() {
                return Err(format!("{what} has no files"));
            }
            for d in v.files.values() {
                check_download(d, p.kind).map_err(|e| format!("{what}: {e}"))?;
            }
        }
    }
    Ok(repo)
}

/// Whether `url` may be read: https:// only, plus http:// in debug builds (to test
/// with a local repository).
fn allowed_scheme(url: &str) -> Result<(), String> {
    if url.starts_with("https://") || (cfg!(debug_assertions) && url.starts_with("http://")) {
        Ok(())
    } else {
        Err(format!("{url} is not an https:// address"))
    }
}

fn check_download(d: &Download, kind: Kind) -> Result<(), String> {
    allowed_scheme(&d.url)?;
    if d.sha256.len() != 64 || !d.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("{}: no valid sha256", d.url));
    }
    let name = d.file_name();
    let safe = !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    let ext = Path::new(name).extension().and_then(|e| e.to_str());
    let ext_ok = match kind {
        Kind::Python => ext.is_some_and(|e| e.eq_ignore_ascii_case("py")),
        Kind::Library => ext.is_some_and(|e| ["dll", "so", "dylib"].contains(&e)),
    };
    if !safe || !ext_ok {
        return Err(format!("{}: not a plugin file name", d.url));
    }
    Ok(())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .into()
}

/// Downloads and checks the repository's list at `url`.
pub fn fetch(url: &str) -> Result<Repository, String> {
    let url = url.trim();
    allowed_scheme(url)?;
    let mut response = agent().get(url).call().map_err(|e| e.to_string())?;
    let json = response
        .body_mut()
        .with_config()
        .limit(MAX_LIST)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    parse(&json)
}

/// Downloads `d` into `dir` (created if need be): the file saved, once its size and
/// SHA-256 are those of the list.
pub fn download(d: &Download, dir: &Path) -> Result<PathBuf, String> {
    let mut response = agent().get(&d.url).call().map_err(|e| e.to_string())?;
    let data = response
        .body_mut()
        .with_config()
        .limit(d.size.saturating_add(1))
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    save(&data, d, dir)
}

/// Saves `data`, the file of `d`, into `dir`, once checked.
pub fn save(data: &[u8], d: &Download, dir: &Path) -> Result<PathBuf, String> {
    use sha2::{Digest, Sha256};
    if data.len() as u64 != d.size {
        return Err(format!(
            "{} bytes downloaded, {} expected",
            data.len(),
            d.size
        ));
    }
    let hash: String = Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if !hash.eq_ignore_ascii_case(&d.sha256) {
        return Err("the file downloaded is not the one listed (SHA-256 differs)".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // The path as the loader makes it, which the user's choices are kept by.
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    let path = dir.join(d.file_name());
    let part = dir.join(format!(".{}.part", d.file_name()));
    std::fs::write(&part, data)
        .and_then(|()| {
            if path.exists() {
                set_aside(&path)?;
            }
            std::fs::rename(&part, &path)
        })
        .map_err(|e| {
            let _ = std::fs::remove_file(&part);
            e.to_string()
        })?;
    Ok(path)
}

/// Moves the plugin file `path` out of the way, to `.<name>.old` beside it (never
/// loaded): a library loaded by the app can't be replaced or deleted on Windows, but can
/// be renamed. The copy set aside before it is deleted.
pub fn set_aside(path: &Path) -> std::io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let old = path.with_file_name(format!(".{name}.old"));
    let _ = std::fs::remove_file(&old);
    std::fs::rename(path, old)
}

/// A local web server for the tests, on a port of its own: each `(path, content)` of
/// `files` at `http://127.0.0.1:<port>/<path>` (debug builds read http://), any other
/// path not found. Returns its address, ending with `/`.
#[cfg(test)]
pub(crate) fn serve(files: Vec<(&'static str, Vec<u8>)>) -> String {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(&stream);
            let mut request = String::new();
            let _ = reader.read_line(&mut request);
            // The headers, up to the blank line.
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let path = request
                .split(' ')
                .nth(1)
                .unwrap_or("/")
                .trim_start_matches('/');
            let mut out = &stream;
            let _ = match files.iter().find(|(p, _)| *p == path) {
                Some((_, body)) => write!(
                    out,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .and_then(|()| out.write_all(body)),
                None => out.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                ),
            };
        }
    });
    base
}

/// The download of `data`, served at `url`, as a repository lists it.
#[cfg(test)]
pub(crate) fn listed(url: String, data: &[u8]) -> Download {
    use sha2::{Digest, Sha256};
    let sha256 = Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Download {
        url,
        sha256,
        size: data.len() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The official repository's list as first published.
    const SAMPLE: &str = r#"{
      "format": 1,
      "name": "Artwork Creator community plugins",
      "plugins": [
        {
          "id": "halftone", "name": "Halftone",
          "description": "Halftone dots effect and dot grid generator",
          "author": "bzhmaddog", "license": "MIT", "kind": "library",
          "versions": [{
            "version": "0.1.0", "released": "2026-11-22", "abi": 2, "min_app": "0.16.0",
            "effects": ["Halftone"], "generators": ["Dot grid"],
            "files": {
              "linux-x86_64": {
                "url": "https://github.com/bzhmaddog/artwork-creator-plugins/releases/download/halftone-0.1.0/libhalftone.so",
                "sha256": "824c1812c9fd2d096e0d2b65f42f0c84bc6cbdae7cb21c4418c81265c2f34ac9",
                "size": 522384
              },
              "windows-x86_64": {
                "url": "https://github.com/bzhmaddog/artwork-creator-plugins/releases/download/halftone-0.1.0/halftone.dll",
                "sha256": "00dadf5d927b92e329aa78d4a7ff0c8cd11f9c60c15446177b7fcf2e0e9d7c59",
                "size": 278016
              }
            }
          }]
        },
        {
          "id": "posterize", "name": "Posterize", "kind": "python",
          "versions": [{
            "version": "1.0.0", "min_app": "0.16.0", "sdk": "0.3.0",
            "files": {"any": {
              "url": "https://github.com/bzhmaddog/artwork-creator-plugins/releases/download/posterize-1.0.0/posterize.py",
              "sha256": "d3e9c742b313113bdb9b65e758436a0297e2846efeadf500dc0b06f9db7a5105",
              "size": 2729
            }}
          }]
        }
      ]
    }"#;

    #[test]
    fn lists_and_files_are_downloaded_and_checked() {
        let file = b"plugin = None\n".to_vec();
        let base = serve(vec![("list.json", SAMPLE.into()), ("a.py", file.clone())]);
        let urls = [
            format!("{base}list.json"),
            format!("{base}gone.json"),
            "ftp://x/l.json".into(),
        ];
        let catalog = fetch_all(&urls);
        assert_eq!(catalog[0].1.as_ref().unwrap().plugins.len(), 2);
        assert!(catalog[1].1.as_ref().unwrap_err().contains("404"));
        assert!(catalog[2].1.as_ref().unwrap_err().contains("https://"));
        let dir = std::env::temp_dir().join(format!("artwork-download-{}", std::process::id()));
        let d = listed(format!("{base}a.py"), &file);
        let path = download(&d, &dir).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), file);
        let wrong = Download {
            sha256: "0".repeat(64),
            ..d.clone()
        };
        assert!(download(&wrong, &dir).unwrap_err().contains("SHA-256"));
        let short = Download { size: 3, ..d };
        assert!(download(&short, &dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_repository_list_is_read() {
        let repo = parse(SAMPLE).unwrap();
        assert_eq!(repo.name, "Artwork Creator community plugins");
        assert_eq!(repo.plugins.len(), 2);
        assert_eq!(repo.plugins[1].kind, Kind::Python);
        let (v, d) = repo.plugins[0]
            .best_for("0.16.0", "windows-x86_64")
            .unwrap();
        assert_eq!(
            (v.version.as_str(), d.file_name()),
            ("0.1.0", "halftone.dll")
        );
        assert_eq!(
            (v.effects.clone(), v.generators.clone()),
            (vec!["Halftone".to_owned()], vec!["Dot grid".to_owned()])
        );
        let (v, d) = repo.plugins[1].best_for("0.16.0", "linux-x86_64").unwrap();
        assert_eq!(d.file_name(), "posterize.py");
        // A list that does not say what a plugin provides is read too.
        assert!(v.effects.is_empty() && v.generators.is_empty());
    }

    #[test]
    fn the_latest_version_that_fits_is_chosen() {
        let mut repo = parse(SAMPLE).unwrap();
        let p = &mut repo.plugins[0];
        // Too new for this app, then for its plugin interface; a version 0.2.0 that fits.
        let mut newer = p.versions[0].clone();
        newer.version = "0.10.0".into();
        newer.min_app = Some("99.0.0".into());
        let mut other_abi = newer.clone();
        other_abi.min_app = None;
        other_abi.abi = Some(abi::ABI_VERSION + 1);
        let mut fits = p.versions[0].clone();
        fits.version = "0.2.0".into();
        p.versions.extend([newer, other_abi, fits]);
        let best = |app, system| p.best_for(app, system).map(|(v, _)| v.version.clone());
        assert_eq!(best("0.16.0", "linux-x86_64").as_deref(), Some("0.2.0"));
        assert_eq!(best("0.15.9", "linux-x86_64"), None);
        assert_eq!(best("0.16.0", "linux-aarch64"), None);
    }

    #[test]
    fn a_bad_list_says_what_is_wrong() {
        let bad = |from: &str, to: &str| parse(&SAMPLE.replacen(from, to, 1)).unwrap_err();
        assert!(bad("\"format\": 1", "\"format\": 2").contains("format 2"));
        assert!(bad("\"kind\": \"library\"", "\"kind\": \"wasm\"").contains("unknown variant"));
        assert!(bad("https://github.com", "ftp://github.com").contains("https://"));
        assert_eq!(
            parse(&SAMPLE.replacen("https://github.com", "http://github.com", 1)).is_ok(),
            cfg!(debug_assertions)
        );
        assert!(bad("824c18", "zz4c18").contains("sha256"));
        assert!(bad("libhalftone.so", "..").contains("file name"));
        assert!(bad("halftone.dll", "halftone.exe").contains("file name"));
        assert!(bad("\"version\": \"1.0.0\"", "\"version\": \"one\"").contains("version"));
        assert!(bad("\"name\": \"Halftone\"", "\"name\": \"\"").contains("no id"));
        assert!(
            bad(
                "\"id\": \"halftone\",",
                "\"id\": \"halftone\", \"latest\": \"new\","
            )
            .contains("latest")
        );
        assert!(bad("\"min_app\": \"0.16.0\"", "\"min_app\": \"soon\"").contains("version"));
        assert!(bad("\"id\": \"posterize\"", "\"id\": \"halftone\"").contains("twice"));
        assert!(parse("not json").is_err());
        assert!(parse("{\"format\": 1, \"name\": \"\", \"plugins\": []}").is_err());
    }

    /// Reads the official repository and downloads its plugins for this system, over the
    /// network: `cargo test fetches_the_official_repository -- --ignored`.
    #[test]
    #[ignore]
    fn fetches_the_official_repository() {
        let repo = fetch(OFFICIAL).unwrap();
        assert!(!repo.plugins.is_empty());
        let dir = std::env::temp_dir().join(format!("artwork-official-{}", std::process::id()));
        for p in &repo.plugins {
            if let Some((_, d)) = p.best() {
                let path = download(d, &dir).unwrap();
                assert_eq!(std::fs::metadata(path).unwrap().len(), d.size);
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn installed_files_are_found_in_the_lists_by_their_content() {
        let mut repo = parse(SAMPLE).unwrap();
        let python = &mut repo.plugins[1];
        let mut newer = python.versions[0].clone();
        newer.version = "1.1.0".into();
        newer.changes = "Faster".into();
        newer.files.get_mut("any").unwrap().sha256 = "1".repeat(64);
        python.versions.push(newer);
        let hash = |h: &str| h.to_owned();
        let files = [
            (
                PathBuf::from("/p/posterize.py"),
                hash("D3E9C742B313113BDB9B65E758436A0297E2846EFEADF500DC0B06F9DB7A5105"),
            ),
            (PathBuf::from("/p/latest.py"), "1".repeat(64)),
            (PathBuf::from("/p/mine.py"), "2".repeat(64)),
        ];
        let known = known(&files, [("https://example.com/plugins.json", &repo)]);
        let old = &known[&files[0].0];
        assert_eq!(
            (old.version.as_str(), old.description.as_str()),
            ("1.0.0", "")
        );
        let update = old.update.as_ref().unwrap();
        assert_eq!(
            (update.version.as_str(), update.changes.as_str()),
            ("1.1.0", "Faster")
        );
        assert_eq!(update.kind, Kind::Python);
        // The latest version has nothing newer; a file no list has is not there.
        assert_eq!(known[&files[1].0].update, None);
        assert!(!known.contains_key(&files[2].0));
    }

    #[test]
    fn an_installed_version_no_longer_listed_is_found_by_its_record() {
        let dir = std::env::temp_dir().join(format!("artwork-record-{}", std::process::id()));
        let url = "https://example.com/plugins.json";
        let origin = dir.join(super::super::plugin::origin(url));
        let entry = |v: &str| super::super::plugin::Installed {
            entry: crate::render::effects::PluginRef {
                id: "halftone".into(),
                version: v.into(),
            },
            compatible_from: None,
        };
        super::super::plugin::Source::record(&origin, "libhalftone.so", entry("0.0.9"), None)
            .unwrap();
        let mut repo = parse(SAMPLE).unwrap();
        let halftone = &mut repo.plugins[0];
        let mut newer = halftone.versions[0].clone();
        newer.version = "1.0.0".into();
        newer.compatible_from = Some("1.0.0".into());
        halftone.versions.push(newer);
        let files = [(origin.join("libhalftone.so"), "2".repeat(64))];
        let update = |repo: &Repository| {
            let known = known(&files, [(url, repo)]);
            let k = &known[&files[0].0];
            assert_eq!(k.version, "0.0.9");
            k.update.as_ref().map(|u| (u.version.clone(), u.breaking))
        };
        assert_eq!(update(&repo), Some(("1.0.0".into(), true)));
        // Not past `latest`; an update that reads what 0.0.9 saved does not warn.
        repo.plugins[0].latest = Some("0.1.0".into());
        assert_eq!(update(&repo), Some(("0.1.0".into(), false)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_version_replaces_the_file_the_old_one_set_aside() {
        let dir = std::env::temp_dir().join(format!("artwork-replace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("p.py"), "old").unwrap();
        let data = b"new";
        let d = Download {
            url: "https://example.com/p.py".into(),
            sha256: crate::render::effects::plugin::hash(data),
            size: 3,
        };
        let path = save(data, &d, &dir).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert_eq!(std::fs::read(dir.join(".p.py.old")).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_download_is_saved_only_when_it_is_the_file_listed() {
        let dir = std::env::temp_dir().join(format!("artwork-repo-{}", std::process::id()));
        let data = b"print('hi')\n";
        let mut d = Download {
            url: "https://example.com/files/hi.py?raw=1".into(),
            sha256: "0".repeat(64),
            size: data.len() as u64,
        };
        assert!(save(data, &d, &dir).unwrap_err().contains("SHA-256"));
        d.sha256 = crate::render::effects::plugin::hash(data);
        d.size += 1;
        assert!(save(data, &d, &dir).unwrap_err().contains("expected"));
        d.size -= 1;
        let path = save(data, &d, &dir).unwrap();
        assert_eq!(path, dir.canonicalize().unwrap().join("hi.py"));
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
