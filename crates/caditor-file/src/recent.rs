use std::{
    ffi::OsString,
    io,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::{read::read_file, save::write_atomically};

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    pub fn load(state_dir: &Path) -> Self {
        let stored = read_file(&state_dir.join(RECENT_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<Value>>(&bytes).ok())
            .unwrap_or_default();
        let mut recent = Self::default();
        for path in stored.iter().rev().filter_map(stored_path) {
            recent.add(path);
        }
        recent
    }

    pub fn save(&self, state_dir: &Path) -> io::Result<()> {
        let stored: Vec<Value> = self.paths.iter().map(|path| storable(path)).collect();
        let contents = serde_json::to_vec_pretty(&stored).map_err(io::Error::other)?;
        std::fs::create_dir_all(state_dir)?;
        write_atomically(&state_dir.join(RECENT_FILE), &contents)
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn add(&mut self, path: PathBuf) {
        self.paths.retain(|existing| *existing != path);
        self.paths.insert(0, path);
        self.paths.truncate(RECENT_LIMIT);
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
}
