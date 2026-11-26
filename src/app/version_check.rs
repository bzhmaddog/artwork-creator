//! Looks for a newer release of the app at startup, in the background: the version in
//! the project's web page's version.json, the latest stable release's, or version-dev.json,
//! the latest release's, pre-release or not, depending on the update channel (both written
//! with the download page by scripts/download-page.py). Nothing is said when it can't be
//! read (offline, a proxy...).

use crate::i18n::tr;
use std::thread::JoinHandle;
use std::time::Duration;

/// The latest stable release's version, as {"version": "1.2.0"}.
const VERSION_FILE: &str = "https://bzhmaddog.github.io/artwork-creator-app/version.json";
/// The latest release's version, pre-release or not, the same way.
const DEV_VERSION_FILE: &str = "https://bzhmaddog.github.io/artwork-creator-app/version-dev.json";
/// Where to get a new version.
pub(super) const DOWNLOAD_PAGE: &str =
    "https://bzhmaddog.github.io/artwork-creator-app/download.html";
/// Longest answer read.
const MAX_ANSWER: u64 = 4096;

/// Where the update channel is kept with the app's preferences.
const STORAGE_KEY: &str = "update_channel";

/// Which new versions the app tells about (Help > Update channel).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum Channel {
    /// Stable releases only.
    #[default]
    Stable,
    /// Every release, pre-releases included.
    Dev,
}

impl Channel {
    const ALL: [Channel; 2] = [Channel::Stable, Channel::Dev];

    /// The version file it reads.
    fn url(self) -> &'static str {
        match self {
            Channel::Stable => VERSION_FILE,
            Channel::Dev => DEV_VERSION_FILE,
        }
    }

    /// Its name in the menu.
    fn label(self) -> &'static str {
        match self {
            Channel::Stable => tr!("UPDATE_CHANNEL_STABLE"),
            Channel::Dev => tr!("UPDATE_CHANNEL_DEV"),
        }
    }

    /// The channel saved by a previous session, or Stable.
    pub(super) fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|s| eframe::get_value(s, STORAGE_KEY))
            .unwrap_or_default()
    }

    pub(super) fn save(self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, &self);
    }
}

/// The latest version of `channel`, when it is newer than this one; nothing for the
/// Microsoft Store package, which the Store updates.
pub(super) fn start(channel: Channel) -> Option<JoinHandle<Option<String>>> {
    start_from(from_store(), channel.url())
}

/// The check of the version file at `url`, unless the app runs from the Microsoft Store.
fn start_from(store: bool, url: &'static str) -> Option<JoinHandle<Option<String>>> {
    (!store).then(|| check(url))
}

/// Help > Update channel, when the app looks for new versions (not in the Microsoft Store
/// package, which the Store updates): which ones to tell about. Whether it was changed.
pub(super) fn help_menu(ui: &mut egui::Ui, channel: &mut Channel) -> bool {
    if from_store() {
        return false;
    }
    let mut changed = false;
    ui.menu_button(tr!("MENU_UPDATE_CHANNEL"), |ui| {
        changed = channel_menu(ui, channel)
    })
    .response
    .on_hover_text(tr!("UPDATE_CHANNEL_HINT"));
    changed
}

/// The channels, the one in use checked; whether another one was chosen.
fn channel_menu(ui: &mut egui::Ui, channel: &mut Channel) -> bool {
    let mut changed = false;
    for c in Channel::ALL {
        changed |= ui.radio_value(channel, c, c.label()).changed();
    }
    changed
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
        // A pre-release (dev channel) is newer than the stable one before it only.
        assert!(is_newer("0.19.0-beta.1", "0.18.0"));
        assert!(!is_newer("0.19.0-beta.1", "0.19.0"));
    }

    #[derive(Default)]
    struct Memory(std::collections::HashMap<String, String>);

    impl eframe::Storage for Memory {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn the_channel_is_kept_between_sessions() {
        assert_eq!(Channel::load(None), Channel::Stable);
        let mut storage = Memory::default();
        assert_eq!(Channel::load(Some(&storage)), Channel::Stable);
        Channel::Dev.save(&mut storage);
        assert_eq!(Channel::load(Some(&storage)), Channel::Dev);
        // Something unreadable: the default.
        storage
            .0
            .insert(STORAGE_KEY.to_owned(), "\"nightly\"".to_owned());
        assert_eq!(Channel::load(Some(&storage)), Channel::Stable);
    }

    #[test]
    fn channels_have_their_own_labels() {
        let labels = Channel::ALL.map(Channel::label);
        assert!(labels.iter().all(|l| !l.is_empty()));
        assert_ne!(labels[0], labels[1]);
    }

    #[test]
    fn the_store_package_does_not_look() {
        #[cfg(not(windows))]
        assert!(!from_store());
        assert!(start_from(true, "http://127.0.0.1:9/version.json").is_none());
        let newer = serve(r#"{"version": "999.0.0"}"#, 1);
        let check = start_from(false, newer).unwrap();
        assert_eq!(check.join().unwrap().as_deref(), Some("999.0.0"));
    }

    /// Runs `ui` for a frame with `events`; the texts drawn and where.
    fn frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mut ui: impl FnMut(&mut egui::Ui),
    ) -> Vec<(String, egui::Rect)> {
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |root| {
            egui::CentralPanel::default().show(root, |panel| ui(panel));
        });
        out.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in &out.shapes {
            if let egui::Shape::Text(t) = &clipped.shape {
                let rect = egui::Rect::from_min_size(t.pos, t.galley.size());
                texts.push((t.galley.text().to_owned(), rect));
            }
        }
        texts
    }

    #[test]
    fn choosing_a_channel_in_the_menu_changes_it() {
        let ctx = egui::Context::default();
        let mut channel = Channel::Stable;
        let mut changed = false;
        let texts = frame(&ctx, vec![], |ui| changed |= channel_menu(ui, &mut channel));
        let labels: Vec<_> = texts.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(labels, Channel::ALL.map(Channel::label));
        assert!(!changed);
        let at = texts[1].1.center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        for events in [
            vec![egui::Event::PointerMoved(at), button(true)],
            vec![button(false)],
        ] {
            frame(&ctx, events, |ui| changed |= channel_menu(ui, &mut channel));
        }
        assert!(changed);
        assert_eq!(channel, Channel::Dev);
    }

    #[test]
    fn the_help_menu_offers_the_channels() {
        let ctx = egui::Context::default();
        let mut channel = Channel::Dev;
        let mut changed = false;
        let texts = frame(&ctx, vec![], |ui| changed |= help_menu(ui, &mut channel));
        assert!(!changed);
        assert_eq!(channel, Channel::Dev);
        // Shown unless run from the Store package.
        let shown = texts.iter().any(|(t, _)| t == tr!("MENU_UPDATE_CHANNEL"));
        assert_eq!(shown, !from_store());
    }

    #[test]
    fn each_channel_reads_its_own_file() {
        assert_eq!(Channel::default(), Channel::Stable);
        assert!(Channel::Stable.url().ends_with("/version.json"));
        assert!(Channel::Dev.url().ends_with("/version-dev.json"));
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
