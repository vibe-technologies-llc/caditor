use std::{
    io,
    path::{Path, PathBuf},
};

use crate::save::write_atomically;

pub const RECENT_LIMIT: usize = 10;
const RECENT_FILE: &str = "recent-files.json";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    pub fn load(state_dir: &Path) -> Self {
        let stored = std::fs::read(state_dir.join(RECENT_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
            .unwrap_or_default();
        let mut recent = Self::default();
        for path in stored.into_iter().rev() {
            recent.add(PathBuf::from(path));
        }
        recent
    }

    pub fn save(&self, state_dir: &Path) -> io::Result<()> {
        let stored: Vec<&str> = self.paths.iter().filter_map(|path| path.to_str()).collect();
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
