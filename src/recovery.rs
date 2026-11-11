//! Autosave and crash recovery.
//!
//! While the project has unsaved changes, a copy of it is written every few minutes
//! (`Autosave`) into a folder of this run's own under the app's data folder, never into
//! the project file. The folder holds a lock file, locked as long as the app runs: the
//! system lets it go when the app ends, crashed or not. The copy is deleted once the
//! project is saved or its changes let go, and the folder when the app closes normally.
//! So a folder whose lock can be taken, holding a copy, was left by an app that crashed
//! (a plugin taking it down, for instance): at the next start, it is offered back
//! (`orphans`).

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::document::Document;
use crate::export::ExportSettings;
use crate::i18n::tr;
use crate::project;

/// The project copy in a session folder: an archive, images included.
const COPY: &str = "project.artwork";
/// What the copy is a copy of (`Info`).
const INFO: &str = "info.json";
/// Locked while the session's app runs.
const LOCK: &str = "lock";
/// A folder with nothing to recover is removed by another run only past this age: the
/// app that made it may not have locked it yet.
const STALE: Duration = Duration::from_secs(60);

/// Minutes between autosaves offered in File > Autosave; 0 is off.
pub const INTERVALS: [u32; 7] = [0, 1, 2, 5, 10, 15, 30];
/// The default interval, in minutes.
pub const DEFAULT_INTERVAL: u32 = 2;
/// Where the interval is kept between sessions.
pub const STORAGE_KEY: &str = "autosave_minutes";

/// The folder holding the sessions' folders: in the app's data folder.
pub fn root() -> Option<PathBuf> {
    eframe::storage_dir("Artwork Creator").map(|d| d.join("recovery"))
}

/// What a copy is a copy of.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Info {
    /// The project file the copy was made from, if it had been saved.
    pub original: Option<PathBuf>,
    /// The project's name, as in the title bar.
    pub name: String,
    /// When the copy was written, in seconds since 1970.
    pub saved_at: u64,
}

impl Info {
    /// When the copy was written.
    pub fn saved_time(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(self.saved_at)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// This run's folder, locked while it runs.
pub struct Session {
    dir: PathBuf,
    /// Holds the lock; dropped to let it go.
    lock: Option<File>,
}

impl Session {
    /// Starts a folder of its own under `root`, locked.
    pub fn start(root: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(root)?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = root.join(format!("{}-{nanos}", std::process::id()));
        fs::create_dir(&dir)?;
        let lock = File::create(dir.join(LOCK))?;
        lock.try_lock().map_err(std::io::Error::other)?;
        Ok(Self {
            dir,
            lock: Some(lock),
        })
    }

    /// Takes over the folder of a crashed run: its copy stays until replaced.
    fn adopt(orphan: Orphan) -> Self {
        Self {
            dir: orphan.dir,
            lock: Some(orphan.lock),
        }
    }

    pub fn copy_path(&self) -> PathBuf {
        self.dir.join(COPY)
    }

    /// Writes `doc` as the copy of the project `info` describes: the archive, then its
    /// info, each replacing the last one whole.
    fn write(&self, doc: &Document, export: &ExportSettings, info: &Info) -> Result<(), String> {
        project::save(doc, export, &self.copy_path())?;
        let json = serde_json::to_string_pretty(info).map_err(|e| e.to_string())?;
        let tmp = self.dir.join(format!("{INFO}.tmp"));
        fs::write(&tmp, json)
            .and_then(|()| fs::rename(&tmp, self.dir.join(INFO)))
            .map_err(|e| e.to_string())
    }

    /// Deletes the copy: there is nothing to recover.
    fn clear(&self) {
        fs::remove_file(self.dir.join(INFO)).ok();
        fs::remove_file(self.copy_path()).ok();
    }

    /// Removes the folder, the app closing normally.
    fn end(mut self) {
        // Windows doesn't delete a file still open.
        drop(self.lock.take());
        fs::remove_dir_all(&self.dir).ok();
    }
}

/// A copy left by a run that ended without closing normally: its folder, locked by this
/// run while it decides.
pub struct Orphan {
    dir: PathBuf,
    lock: File,
    pub info: Info,
}

impl Orphan {
    pub fn copy_path(&self) -> PathBuf {
        self.dir.join(COPY)
    }

    /// Deletes it, its work let go.
    pub fn discard(self) {
        drop(self.lock);
        fs::remove_dir_all(&self.dir).ok();
    }
}

/// The copies under `root` left by runs that crashed, newest first. Folders of crashed
/// runs with nothing to recover are removed; those of runs still going are left alone.
pub fn orphans(root: &Path) -> Vec<Orphan> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dir in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        let lock_path = dir.join(LOCK);
        let Ok(lock) = OpenOptions::new().write(true).open(&lock_path) else {
            continue;
        };
        // Held: its app is running.
        if lock.try_lock().is_err() {
            continue;
        }
        let info = fs::read(dir.join(INFO))
            .ok()
            .and_then(|b| serde_json::from_slice::<Info>(&b).ok())
            .filter(|_| dir.join(COPY).is_file());
        match info {
            Some(info) => out.push(Orphan { dir, lock, info }),
            None => {
                let old = fs::metadata(&lock_path)
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > STALE));
                drop(lock);
                if old {
                    fs::remove_dir_all(&dir).ok();
                }
            }
        }
    }
    out.sort_by_key(|o| std::cmp::Reverse(o.info.saved_at));
    out
}

/// The project as it is now, as `Autosave::tick` is told it.
pub struct Project<'a> {
    pub doc: &'a Document,
    /// What the project file holds of it, to tell whether it changed since the last
    /// copy (`History::key`).
    pub key: &'a str,
    pub export: &'a ExportSettings,
    pub path: Option<&'a Path>,
    pub name: String,
    /// It has unsaved changes.
    pub modified: bool,
}

/// Writes a copy of the project every `minutes` while it has unsaved changes.
pub struct Autosave {
    /// Minutes between copies; 0 is off.
    pub minutes: u32,
    /// Under which the session's folder goes; none when there is no data folder.
    root: Option<PathBuf>,
    /// Started at the first copy.
    session: Option<Session>,
    /// What the copy on disk holds, or is being written: the key and export choices.
    copied: Option<(String, ExportSettings)>,
    /// When the next copy is due, the project having changed since the last.
    due: Option<Instant>,
    job: Option<JoinHandle<Result<(), String>>>,
    /// The last copy failed: reported once, until one works.
    failed: bool,
    /// When the copy on disk was written, while there is one.
    pub last_copy: Option<SystemTime>,
}

impl Autosave {
    pub fn new(root: Option<PathBuf>, minutes: u32) -> Self {
        Self {
            minutes,
            root,
            session: None,
            copied: None,
            due: None,
            job: None,
            failed: false,
            last_copy: None,
        }
    }

    /// Writes a copy when one is due, or deletes it when there is nothing to recover.
    /// `busy` holds a copy back (a drag going on). Returns an error to show, once, and
    /// asks for a frame when the next copy is due.
    pub fn tick(&mut self, ctx: &egui::Context, p: Project, busy: bool) -> Option<String> {
        let mut error = None;
        if self.job.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(job) = self.job.take()
        {
            match job.join().unwrap_or_else(|_| Err("panic".into())) {
                Ok(()) => {
                    self.failed = false;
                    self.last_copy = Some(SystemTime::now());
                }
                Err(e) => {
                    // Written again at the next change.
                    self.copied = None;
                    if !std::mem::replace(&mut self.failed, true) {
                        error = Some(tr!("AUTOSAVE_FAILED", e = e));
                    }
                }
            }
        }
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
            return error;
        }
        if !p.modified || self.minutes == 0 {
            self.due = None;
            if self.copied.take().is_some()
                && let Some(s) = &self.session
            {
                s.clear();
                self.last_copy = None;
            }
            return error;
        }
        if self
            .copied
            .as_ref()
            .is_some_and(|(key, export)| key == p.key && export.same_choices(p.export))
        {
            self.due = None;
            return error;
        }
        let now = Instant::now();
        let due = *self
            .due
            .get_or_insert(now + Duration::from_secs(self.minutes as u64 * 60));
        if now < due || busy {
            ctx.request_repaint_after(
                due.saturating_duration_since(now)
                    .max(Duration::from_millis(200)),
            );
            return error;
        }
        self.due = None;
        if self.session.is_none() {
            let Some(root) = &self.root else {
                return error;
            };
            match Session::start(root) {
                Ok(s) => self.session = Some(s),
                Err(e) => {
                    // Tried again at the next change.
                    self.copied = Some((p.key.to_owned(), p.export.clone()));
                    if !std::mem::replace(&mut self.failed, true) {
                        error = Some(tr!("AUTOSAVE_FAILED", e = e));
                    }
                    return error;
                }
            }
        }
        let Some(session) = &self.session else {
            return error;
        };
        self.copied = Some((p.key.to_owned(), p.export.clone()));
        let (doc, export) = (p.doc.clone(), p.export.clone());
        let info = Info {
            original: p.path.map(Path::to_path_buf),
            name: p.name,
            saved_at: now_secs(),
        };
        let dir = session.dir.clone();
        self.job = Some(std::thread::spawn(move || {
            let session = Session { dir, lock: None };
            session.write(&doc, &export, &info)
        }));
        ctx.request_repaint_after(Duration::from_millis(200));
        error
    }

    /// The project was recovered from `orphan`, as `key` and `export`: its copy becomes
    /// this run's, so it is not lost if the app crashes before the next one.
    pub fn adopt(&mut self, orphan: Orphan, key: &str, export: &ExportSettings) {
        match &self.session {
            None => {
                self.last_copy = Some(orphan.info.saved_time());
                self.session = Some(Session::adopt(orphan));
                self.copied = Some((key.to_owned(), export.clone()));
            }
            // Copied again as soon as can be.
            Some(_) => {
                self.copied = None;
                self.due = Some(Instant::now());
                orphan.discard();
            }
        }
    }

    /// The app closes normally: once the copy being written is done, the folder goes.
    pub fn end(&mut self) {
        if let Some(job) = self.job.take() {
            job.join().ok();
        }
        if let Some(s) = self.session.take() {
            s.end();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("artwork-recovery-{name}-{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        root
    }

    fn info(name: &str) -> Info {
        Info {
            original: Some(PathBuf::from(format!("/projects/{name}.artwork"))),
            name: name.into(),
            saved_at: now_secs(),
        }
    }

    #[test]
    fn a_crashed_runs_copy_is_found_and_a_running_ones_not() {
        let root = temp_root("orphans");
        let doc = Document::default();
        let export = ExportSettings::default();
        // A run still going: its folder is locked.
        let running = Session::start(&root).unwrap();
        running.write(&doc, &export, &info("running")).unwrap();
        // A crashed run: its lock let go, its copy left.
        let crashed = Session::start(&root).unwrap();
        crashed.write(&doc, &export, &info("crashed")).unwrap();
        drop(crashed.lock);
        // A run that crashed before its first copy: nothing to recover.
        let empty = Session::start(&root).unwrap();
        let empty_dir = empty.dir.clone();
        drop(empty.lock);

        let found = orphans(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].info, info("crashed"));
        let (doc2, _) = project::load(&found[0].copy_path()).unwrap();
        assert_eq!(doc2.layers.len(), doc.layers.len());
        // Found, it is locked: another run starting now doesn't offer it too.
        assert!(orphans(&root).is_empty());
        // Too recent to be removed: its app may be about to lock it.
        assert!(empty_dir.is_dir());

        // Discarded, it is gone; a normal close removes the running one's folder.
        let dir = found[0].dir.clone();
        found.into_iter().for_each(Orphan::discard);
        assert!(!dir.exists());
        let running_dir = running.dir.clone();
        running.end();
        assert!(!running_dir.exists());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn autosave_copies_changes_then_deletes_the_copy_once_saved() {
        let root = temp_root("autosave");
        let ctx = egui::Context::default();
        let doc = Document::default();
        let export = ExportSettings::default();
        let mut autosave = Autosave::new(Some(root.clone()), 1);
        let project = |key: &'static str, modified: bool| Project {
            doc: &doc,
            key,
            export: &export,
            path: None,
            name: "Untitled".into(),
            modified,
        };
        let wait = |a: &mut Autosave| {
            while a.job.as_ref().is_some_and(|j| !j.is_finished()) {
                std::thread::yield_now();
            }
        };
        // Changed: due in a minute, nothing written yet.
        assert!(autosave.tick(&ctx, project("a", true), false).is_none());
        assert!(autosave.session.is_none());
        // Due: written.
        autosave.due = Some(Instant::now());
        autosave.tick(&ctx, project("a", true), false);
        wait(&mut autosave);
        autosave.tick(&ctx, project("a", true), false);
        let copy = autosave.session.as_ref().unwrap().copy_path();
        assert!(copy.is_file());
        // Not changed since: nothing due.
        assert!(autosave.due.is_none());
        // Changed, but a drag goes on: held back.
        autosave.tick(&ctx, project("b", true), false);
        autosave.due = Some(Instant::now());
        autosave.tick(&ctx, project("b", true), true);
        assert!(autosave.job.is_none());
        // Saved: the copy goes, the folder stays until the app closes.
        autosave.tick(&ctx, project("b", false), false);
        assert!(!copy.exists());
        let dir = autosave.session.as_ref().unwrap().dir.clone();
        assert!(dir.is_dir());
        autosave.end();
        assert!(!dir.exists());
        fs::remove_dir_all(&root).ok();
    }
}
