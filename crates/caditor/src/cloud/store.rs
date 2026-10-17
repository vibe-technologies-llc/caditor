use std::{
    fs, io,
    path::{Path, PathBuf},
};

use caditor_cloud::{Service, Session};

const FOLDER: &str = "cloud";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    pub fn in_state_dir(state_dir: &Path, service: Service) -> Self {
        Self {
            path: state_dir
                .join(FOLDER)
                .join(format!("{}-session.json", service.id())),
        }
    }

    pub fn downloads_dir(state_dir: &Path) -> PathBuf {
        state_dir.join(FOLDER).join("downloads")
    }

    pub fn load(&self) -> io::Result<Option<Session>> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        match serde_json::from_slice(&bytes) {
            Ok(session) => Ok(Some(session)),
            Err(error) => {
                log::warn!(
                    "the saved {} sign-in is damaged and was set aside: {error}",
                    self.path.display()
                );
                self.forget()?;
                Ok(None)
            }
        }
    }

    pub fn save(&self, session: &Session) -> io::Result<()> {
        if let Some(folder) = self.path.parent() {
            fs::create_dir_all(folder)?;
        }
        let bytes = serde_json::to_vec(session).map_err(io::Error::other)?;
        caditor_file::write_privately(&self.path, &bytes)
    }

    pub fn forget(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn session() -> Session {
        Session {
            access_token: "a".to_owned(),
            refresh_token: Some("r".to_owned()),
            expires_at: 9,
        }
    }

    #[test]
    fn nothing_saved_loads_as_no_session() {
        let dir = TempDir::new().unwrap();

        let store = SessionStore::in_state_dir(dir.path(), Service::Fusion);

        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn a_saved_session_loads_again_and_forgetting_removes_it() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::in_state_dir(dir.path(), Service::Fusion);

        store.save(&session()).unwrap();

        assert_eq!(store.load().unwrap(), Some(session()));

        store.forget().unwrap();
        store.forget().unwrap();

        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn a_damaged_file_is_dropped_rather_than_failing_every_start() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::in_state_dir(dir.path(), Service::Fusion);
        store.save(&session()).unwrap();
        fs::write(&store.path, b"not json").unwrap();

        assert_eq!(store.load().unwrap(), None);
        assert!(!store.path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_saved_session_is_readable_by_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let store = SessionStore::in_state_dir(dir.path(), Service::Fusion);

        store.save(&session()).unwrap();

        let mode = fs::metadata(&store.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }
}
