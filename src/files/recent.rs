//! Recently opened or saved projects, newest first, kept with the app's settings.

use std::path::{Path, PathBuf};

/// How many projects the list keeps.
const MAX_RECENT: usize = 10;
/// Key in eframe's storage.
const STORAGE_KEY: &str = "recent_projects";

#[derive(Default)]
pub struct RecentFiles {
    /// Absolute paths, newest first, no duplicates.
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    /// The list saved by a previous session, or an empty one.
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let paths: Vec<PathBuf> = storage
            .and_then(|s| eframe::get_value(s, STORAGE_KEY))
            .unwrap_or_default();
        let mut recent = Self::default();
        // Oldest first, so the newest ends up on top.
        for p in paths.iter().rev() {
            recent.add(p);
        }
        recent
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, &self.paths);
    }

    /// Puts a project at the top of the list.
    pub fn add(&mut self, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.paths.retain(|p| *p != path);
        self.paths.insert(0, path);
        self.paths.truncate(MAX_RECENT);
    }

    /// Drops a project, e.g. one that could not be opened.
    pub fn remove(&mut self, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.paths.retain(|p| *p != path);
    }

    pub fn clear(&mut self) {
        self.paths.clear();
    }

    /// Newest first.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }
}

/// Menu label of a project: its file name without the project extensions.
pub fn label(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    name.trim_end_matches(".json")
        .trim_end_matches(".artwork")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
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

    fn abs(p: &str) -> PathBuf {
        std::path::absolute(p).unwrap()
    }

    #[test]
    fn newest_first_without_duplicates_and_capped() {
        let mut r = RecentFiles::default();
        r.add(Path::new("a.artwork.json"));
        r.add(Path::new("b.artwork.json"));
        r.add(Path::new("a.artwork.json"));
        assert_eq!(r.paths(), [abs("a.artwork.json"), abs("b.artwork.json")]);
        for i in 0..20 {
            r.add(Path::new(&format!("p{i}.json")));
        }
        assert_eq!(r.paths().len(), MAX_RECENT);
        assert_eq!(r.paths()[0], abs("p19.json"));
        r.remove(Path::new("p19.json"));
        assert_eq!(r.paths()[0], abs("p18.json"));
    }

    #[test]
    fn kept_between_sessions() {
        let mut storage = MemoryStorage::default();
        let mut r = RecentFiles::default();
        r.add(Path::new("old.json"));
        r.add(Path::new("new.json"));
        r.save(&mut storage);
        let back = RecentFiles::load(Some(&storage));
        assert_eq!(back.paths(), [abs("new.json"), abs("old.json")]);
        assert!(RecentFiles::load(None).paths().is_empty());
    }

    #[test]
    fn labels_drop_project_extensions() {
        assert_eq!(label(Path::new("/x/My stick.artwork.json")), "My stick");
        assert_eq!(label(Path::new("test.json")), "test");
    }
}
