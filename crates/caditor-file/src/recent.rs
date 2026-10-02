use std::{
    ffi::OsString,
    io,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::{
    lock::locked_update,
    read::read_file,
    save::{keep_unreadable, write_atomically},
};

fn storable(path: &Path) -> Value {
    match path.to_str() {
        Some(text) => Value::String(text.to_owned()),
        None => Value::Array(
            path.as_os_str()
                .as_bytes()
                .iter()
                .map(|byte| Value::from(*byte))
                .collect(),
        ),
    }
}

fn stored_path(value: &Value) -> Option<PathBuf> {
    match value {
        Value::String(text) => Some(PathBuf::from(text)),
        Value::Array(bytes) => bytes
            .iter()
            .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
            .collect::<Option<Vec<u8>>>()
            .map(|bytes| PathBuf::from(OsString::from_vec(bytes))),
        _ => None,
    }
}

pub const RECENT_LIMIT: usize = 10;
const RECENT_FILE: &str = "recent-files.json";
const UNREADABLE_STEM: &str = "recent-files.unreadable";
const LOCK_FILE: &str = "recent-files.lock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecentChange {
    Opened(PathBuf),
    Forgotten(PathBuf),
    Cleared,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    pub fn load(state_dir: &Path) -> Self {
        read_file(&state_dir.join(RECENT_FILE))
            .map(|bytes| Self::parse(&bytes))
            .unwrap_or_default()
    }

    pub(crate) fn parse(bytes: &[u8]) -> Self {
        serde_json::from_slice::<Vec<Value>>(bytes)
            .map(|stored| Self::from_stored(&stored))
            .unwrap_or_default()
    }

    fn from_stored(stored: &[Value]) -> Self {
        let mut recent = Self::default();
        for path in stored.iter().rev().filter_map(stored_path) {
            recent.add(path);
        }
        recent
    }

    pub fn save_changes(state_dir: &Path, changes: &[RecentChange]) -> io::Result<()> {
        locked_update(state_dir, LOCK_FILE, || {
            let mut stored = Self::load_keeping_unreadable(state_dir)?;
            for change in changes {
                stored.apply(change);
            }
            stored.write(state_dir)
        })
    }

    fn load_keeping_unreadable(state_dir: &Path) -> io::Result<Self> {
        let path = state_dir.join(RECENT_FILE);
        match read_file(&path) {
            Ok(bytes) => match serde_json::from_slice::<Vec<Value>>(&bytes) {
                Ok(stored) => Ok(Self::from_stored(&stored)),
                Err(error) => {
                    let kept = keep_unreadable(&path, UNREADABLE_STEM)?;
                    log::warn!(
                        "the recent files in {} were unreadable ({error}) and were kept as {}",
                        path.display(),
                        kept.display()
                    );
                    Ok(Self::default())
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn apply(&mut self, change: &RecentChange) {
        match change {
            RecentChange::Opened(path) => self.add(path.clone()),
            RecentChange::Forgotten(path) => self.remove(path),
            RecentChange::Cleared => self.clear(),
        }
    }

    pub fn save(&self, state_dir: &Path) -> io::Result<()> {
        locked_update(state_dir, LOCK_FILE, || {
            Self::load_keeping_unreadable(state_dir)?;
            self.write(state_dir)
        })
    }

    fn write(&self, state_dir: &Path) -> io::Result<()> {
        write_atomically(&state_dir.join(RECENT_FILE), &self.stored()?)
    }

    pub(crate) fn stored(&self) -> io::Result<Vec<u8>> {
        let stored: Vec<Value> = self.paths.iter().map(|path| storable(path)).collect();
        serde_json::to_vec_pretty(&stored).map_err(io::Error::other)
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn add(&mut self, path: PathBuf) {
        self.paths.retain(|existing| *existing != path);
        self.paths.insert(0, path);
        self.paths.truncate(RECENT_LIMIT);
    }

    pub fn clear(&mut self) {
        self.paths.clear();
    }

    pub fn remove(&mut self, path: &Path) {
        self.paths.retain(|existing| existing != path);
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn paths_that_are_not_utf8_are_remembered() {
        let dir = TempDir::new().unwrap();
        let odd = PathBuf::from(OsString::from_vec(b"/models/caf\xe9.caditor".to_vec()));
        let plain = PathBuf::from("/models/plate.caditor");
        let mut recent = RecentFiles::default();
        recent.add(odd.clone());
        recent.add(plain.clone());
        recent.save(dir.path()).unwrap();
        assert_eq!(RecentFiles::load(dir.path()).paths(), [plain, odd]);
    }

    #[test]
    fn an_unreadable_list_is_kept_aside_before_it_is_replaced() {
        let dir = TempDir::new().unwrap();
        let opened = PathBuf::from("/models/plate.caditor");
        std::fs::write(dir.path().join(RECENT_FILE), "[ broken").unwrap();

        RecentFiles::save_changes(dir.path(), &[RecentChange::Opened(opened.clone())]).unwrap();

        assert_eq!(RecentFiles::load(dir.path()).paths(), [opened]);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("recent-files.unreadable.json")).unwrap(),
            "[ broken"
        );

        std::fs::write(dir.path().join(RECENT_FILE), "{}").unwrap();
        RecentFiles::default().save(dir.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("recent-files.unreadable-2.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn windows_updating_at_once_lose_nothing() {
        let dir = TempDir::new().unwrap();
        let windows: Vec<_> = (0..8)
            .map(|index| {
                let state = dir.path().to_path_buf();
                std::thread::spawn(move || {
                    let path = PathBuf::from(format!("/models/{index}.caditor"));
                    RecentFiles::save_changes(&state, &[RecentChange::Opened(path)]).unwrap();
                })
            })
            .collect();
        for window in windows {
            window.join().unwrap();
        }

        assert_eq!(RecentFiles::load(dir.path()).paths().len(), 8);
    }

    #[test]
    fn two_windows_keep_each_others_recent_files() {
        let dir = TempDir::new().unwrap();
        let first = PathBuf::from("/models/first.caditor");
        let second = PathBuf::from("/models/second.caditor");
        let old = PathBuf::from("/models/old.caditor");
        RecentFiles::save_changes(dir.path(), &[RecentChange::Opened(old.clone())]).unwrap();

        RecentFiles::save_changes(dir.path(), &[RecentChange::Opened(first.clone())]).unwrap();
        RecentFiles::save_changes(
            dir.path(),
            &[
                RecentChange::Opened(second.clone()),
                RecentChange::Forgotten(old),
            ],
        )
        .unwrap();

        assert_eq!(RecentFiles::load(dir.path()).paths(), [second, first]);
    }

    #[test]
    fn clearing_forgets_every_recent_file_on_disk_too() {
        let dir = TempDir::new().unwrap();
        let first = PathBuf::from("/models/first.caditor");
        let second = PathBuf::from("/models/second.caditor");
        RecentFiles::save_changes(
            dir.path(),
            &[
                RecentChange::Opened(first.clone()),
                RecentChange::Opened(second),
            ],
        )
        .unwrap();

        RecentFiles::save_changes(dir.path(), &[RecentChange::Cleared]).unwrap();
        assert!(RecentFiles::load(dir.path()).paths().is_empty());

        RecentFiles::save_changes(dir.path(), &[RecentChange::Opened(first.clone())]).unwrap();
        assert_eq!(RecentFiles::load(dir.path()).paths(), [first]);
    }
}
