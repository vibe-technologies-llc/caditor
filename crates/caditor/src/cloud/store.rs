use std::{
    fs, io,
    path::{Path, PathBuf},
};

use caditor_cloud::{Service, Session, WorkspaceId};
use serde::{Deserialize, Serialize};

const FOLDER: &str = "cloud";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStore {
    path: PathBuf,
    hub_path: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct RememberedHub {
    workspace: String,
}

impl SessionStore {
    pub fn in_state_dir(state_dir: &Path, service: Service) -> Self {
        let folder = state_dir.join(FOLDER);
        Self {
            path: folder.join(format!("{}-session.json", service.id())),
            hub_path: folder.join(format!("{}-hub.json", service.id())),
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
        remove_if_present(&self.path)
    }

    pub fn remembered_hub(&self) -> Option<WorkspaceId> {
        let bytes = fs::read(&self.hub_path).ok()?;
        let remembered: RememberedHub = serde_json::from_slice(&bytes).ok()?;
        (!remembered.workspace.is_empty()).then(|| WorkspaceId::new(remembered.workspace))
    }

    pub fn remember_hub(&self, workspace: &WorkspaceId) -> io::Result<()> {
        if let Some(folder) = self.hub_path.parent() {
            fs::create_dir_all(folder)?;
        }
        let bytes = serde_json::to_vec(&RememberedHub {
            workspace: workspace.as_str().to_owned(),
        })
        .map_err(io::Error::other)?;
        caditor_file::write_atomically(&self.hub_path, &bytes)
    }

    pub fn forget_hub(&self) -> io::Result<()> {
        remove_if_present(&self.hub_path)
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
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

    #[test]
    fn a_remembered_hub_loads_again_and_forgetting_removes_it() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::in_state_dir(dir.path(), Service::Fusion);

        assert_eq!(store.remembered_hub(), None);

        store.remember_hub(&WorkspaceId::new("hub-2")).unwrap();

        assert_eq!(store.remembered_hub(), Some(WorkspaceId::new("hub-2")));

        store.forget_hub().unwrap();
        store.forget_hub().unwrap();

        assert_eq!(store.remembered_hub(), None);
    }

    #[test]
    fn a_damaged_or_empty_hub_file_is_no_hub() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::in_state_dir(dir.path(), Service::Fusion);
        store.remember_hub(&WorkspaceId::new("hub-2")).unwrap();

        fs::write(&store.hub_path, b"not json").unwrap();
        assert_eq!(store.remembered_hub(), None);

        fs::write(&store.hub_path, br#"{"workspace":""}"#).unwrap();
        assert_eq!(store.remembered_hub(), None);
    }

    #[test]
    fn each_service_remembers_its_own_hub() {
        let dir = TempDir::new().unwrap();
        let fusion = SessionStore::in_state_dir(dir.path(), Service::Fusion);
        let onshape = SessionStore::in_state_dir(dir.path(), Service::Onshape);

        fusion.remember_hub(&WorkspaceId::new("hub-2")).unwrap();

        assert_eq!(onshape.remembered_hub(), None);
    }
}
