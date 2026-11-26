//! Looks for a newer release of the app at startup, in the background: the version in
//! the project's web page's version.json, the latest release's (written with the download
//! page by scripts/download-page.py). Nothing is said when it can't be read (offline, a
//! proxy...).

use std::thread::JoinHandle;
use std::time::Duration;

/// The latest release's version, as {"version": "1.2.0"}.
const VERSION_FILE: &str = "https://bzhmaddog.github.io/artwork-creator-app/version.json";
/// Where to get a new version.
pub(super) const DOWNLOAD_PAGE: &str =
    "https://bzhmaddog.github.io/artwork-creator-app/download.html";
/// Longest answer read.
const MAX_ANSWER: u64 = 4096;

/// The latest version, when it is newer than this one; nothing for the Microsoft Store
/// package, which the Store updates.
pub(super) fn start() -> Option<JoinHandle<Option<String>>> {
    if from_store() {
        return None;
    }
    Some(check(VERSION_FILE))
}

/// Reads the version file at `url` in the background: its version, when it is newer than
/// this one.
fn check(url: &'static str) -> JoinHandle<Option<String>> {
    std::thread::spawn(move || {
        let latest = latest(url)
            .map_err(|e| log::info!("no version check: {e}"))
            .ok()?;
        is_newer(&latest, env!("CARGO_PKG_VERSION")).then_some(latest)
    })
}

/// Whether the app runs from an MSIX package: the Microsoft Store's.
#[cfg(windows)]
fn from_store() -> bool {
    use windows_sys::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
    use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    let mut len = 0u32;
    // SAFETY: with a length of 0, only the length needed is written; no name is.
    let r = unsafe { GetCurrentPackageFullName(&mut len, std::ptr::null_mut()) };
    r != APPMODEL_ERROR_NO_PACKAGE
}

#[cfg(not(windows))]
fn from_store() -> bool {
    false
}

/// The latest release's version.
fn latest(url: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    let json = response
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    version_of(&json).ok_or_else(|| "no version in the answer".to_owned())
}

/// The version a version.json gives.
fn version_of(json: &str) -> Option<String> {
    let file: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(file["version"].as_str()?.to_owned())
}

/// Whether `latest` is a later version than `current`; false if either isn't one.
fn is_newer(latest: &str, current: &str) -> bool {
    match (
        semver::Version::parse(latest),
        semver::Version::parse(current),
    ) {
        (Ok(l), Ok(c)) => l > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_release_is_read_and_compared() {
        assert_eq!(
            version_of(r#"{"version": "0.18.0"}"#).as_deref(),
            Some("0.18.0")
        );
        assert_eq!(version_of("{}"), None);
        assert_eq!(version_of("not json"), None);
        assert!(is_newer("0.18.0", "0.17.0"));
        assert!(is_newer("1.0.0", "0.17.3"));
        assert!(!is_newer("0.17.0", "0.17.0"));
        assert!(!is_newer("0.16.2", "0.17.0"));
        assert!(!is_newer("nightly", "0.17.0"));
    }

    /// A server on this computer answering `body` to each request, as many times as asked;
    /// its URL.
    fn serve(body: &'static str, times: usize) -> &'static str {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/version.json", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().take(times) {
                let mut stream = stream.unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(answer.as_bytes());
            }
        });
        Box::leak(url.into_boxed_str())
    }

    #[test]
    fn a_newer_release_online_is_told() {
        let newer = serve(r#"{"version": "999.0.0"}"#, 1);
        assert_eq!(check(newer).join().unwrap().as_deref(), Some("999.0.0"));
        let same = serve(r#"{"version": "0.0.1"}"#, 1);
        assert_eq!(check(same).join().unwrap(), None);
        let none = serve("{}", 1);
        assert_eq!(latest(none), Err("no version in the answer".to_owned()));
        // Nothing listening: nothing said.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/version.json", closed.local_addr().unwrap());
        drop(closed);
        assert_eq!(check(Box::leak(url.into_boxed_str())).join().unwrap(), None);
    }
}
