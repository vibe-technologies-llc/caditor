use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use caditor_cloud::{FolderId, FolderRef, ProjectId, Service, Session, WorkspaceId};
use serde::{Deserialize, Serialize};

use super::{
    keyring::{Keyring, KeyringError},
    state::Remembered,
};

const FOLDER: &str = "cloud";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the saved sign-in file could not be removed ({0})")]
    File(io::Error),
    #[error("the keyring would not let go of the saved sign-in ({0})")]
    Keyring(KeyringError),
}

#[derive(Clone)]
pub struct SessionStore {
    path: PathBuf,
    hub_path: PathBuf,
    destination_path: PathBuf,
    thumbnails_dir: PathBuf,
    service: Service,
    keyring: Arc<dyn Keyring>,
}

#[derive(Serialize, Deserialize)]
struct RememberedHub {
    workspace: String,
}

#[derive(Serialize, Deserialize)]
struct RememberedDestination {
    workspace: String,
    project: String,
    folder: String,
    folder_name: String,
}

#[derive(Serialize, Deserialize, Default)]
struct RememberedDestinations {
    #[serde(default)]
    destinations: Vec<RememberedDestination>,
}

impl fmt::Debug for SessionStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionStore")
            .field("path", &self.path)
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

impl SessionStore {
    pub fn in_state_dir(state_dir: &Path, service: Service, keyring: Arc<dyn Keyring>) -> Self {
        let folder = state_dir.join(FOLDER);
        Self {
            path: folder.join(format!("{}-session.json", service.id())),
            hub_path: folder.join(format!("{}-hub.json", service.id())),
            destination_path: folder.join(format!("{}-destination.json", service.id())),
            thumbnails_dir: folder.join("thumbnails").join(service.id()),
            service,
            keyring,
        }
    }

    pub fn thumbnails_dir(&self) -> &Path {
        &self.thumbnails_dir
    }

    pub fn downloads_dir(state_dir: &Path) -> PathBuf {
        state_dir.join(FOLDER).join("downloads")
    }

    pub fn outgoing_dir(state_dir: &Path) -> PathBuf {
        state_dir.join(FOLDER).join("outgoing")
    }

    pub fn load(&self) -> io::Result<Option<Session>> {
        if let Some(session) = self.load_file()? {
            return Ok(Some(session));
        }
        match self.keyring.get(self.service) {
            Ok(Some(bytes)) => Ok(self.parse_kept(&bytes)),
            Ok(None) => Ok(None),
            Err(error) => {
                log_unreachable("read the saved sign-in", &error);
                Ok(None)
            }
        }
    }

    pub fn save(&self, session: &Session) -> io::Result<()> {
        let bytes = serde_json::to_vec(session).map_err(io::Error::other)?;
        match self.keyring.set(self.service, &bytes) {
            Ok(()) => self.remove_file(),
            Err(error) => {
                log_unreachable("keep the sign-in", &error);
                self.write_file(&bytes)
            }
        }
    }

    pub fn forget(&self) -> Result<(), StoreError> {
        let file = self.remove_file().map_err(StoreError::File);
        let kept = match self.keyring.delete(self.service) {
            Err(error) if !error.is_absent() => Err(StoreError::Keyring(error)),
            _ => Ok(()),
        };
        file.and(kept)
    }

    fn load_file(&self) -> io::Result<Option<Session>> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        match serde_json::from_slice(&bytes) {
            Ok(session) => {
                self.move_into_keyring(&bytes);
                Ok(Some(session))
            }
            Err(error) => {
                log::warn!(
                    "the saved {} sign-in is damaged and was set aside: {error}",
                    self.path.display()
                );
                self.remove_file()?;
                Ok(None)
            }
        }
    }

    fn move_into_keyring(&self, bytes: &[u8]) {
        match self.keyring.set(self.service, bytes) {
            Ok(()) => {
                if let Err(error) = self.remove_file() {
                    log::warn!(
                        "the sign-in moved into the keyring but its file could not be removed: {error}"
                    );
                }
            }
            Err(error) => log_unreachable("move the sign-in into it", &error),
        }
    }

    fn parse_kept(&self, bytes: &[u8]) -> Option<Session> {
        match serde_json::from_slice(bytes) {
            Ok(session) => Some(session),
            Err(error) => {
                log::warn!("the sign-in kept in the keyring is damaged and was removed: {error}");
                if let Err(error) = self.keyring.delete(self.service) {
                    log::warn!(
                        "the damaged sign-in could not be removed from the keyring: {error}"
                    );
                }
                None
            }
        }
    }

    fn write_file(&self, bytes: &[u8]) -> io::Result<()> {
        if let Some(folder) = self.path.parent() {
            fs::create_dir_all(folder)?;
        }
        caditor_file::write_privately(&self.path, bytes)
    }

    fn remove_file(&self) -> io::Result<()> {
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

    pub fn remembered_destinations(&self) -> Vec<Remembered> {
        let Ok(bytes) = fs::read(&self.destination_path) else {
            return Vec::new();
        };
        let kept: RememberedDestinations = serde_json::from_slice(&bytes).unwrap_or_default();
        kept.destinations
            .into_iter()
            .filter(|kept| !kept.workspace.is_empty() && !kept.folder.is_empty())
            .map(|kept| Remembered {
                workspace: WorkspaceId::new(kept.workspace),
                folder: FolderRef {
                    project: ProjectId::new(kept.project),
                    folder: FolderId::new(kept.folder),
                },
                folder_name: kept.folder_name,
            })
            .collect()
    }

    pub fn remember_destinations(&self, destinations: &[Remembered]) -> io::Result<()> {
        if destinations.is_empty() {
            return remove_if_present(&self.destination_path);
        }
        if let Some(folder) = self.destination_path.parent() {
            fs::create_dir_all(folder)?;
        }
        let kept = RememberedDestinations {
            destinations: destinations
                .iter()
                .map(|remembered| RememberedDestination {
                    workspace: remembered.workspace.as_str().to_owned(),
                    project: remembered.folder.project.as_str().to_owned(),
                    folder: remembered.folder.folder.as_str().to_owned(),
                    folder_name: remembered.folder_name.clone(),
                })
                .collect(),
        };
        let bytes = serde_json::to_vec(&kept).map_err(io::Error::other)?;
        caditor_file::write_atomically(&self.destination_path, &bytes)
    }

    pub fn forget_destinations(&self) -> io::Result<()> {
        remove_if_present(&self.destination_path)
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn log_unreachable(attempt: &str, error: &KeyringError) {
    if error.is_absent() {
        log::debug!("no keyring to {attempt} ({error}); the owner-only file is used");
    } else {
        log::warn!("the keyring would not {attempt} ({error}); the owner-only file is used");
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::cloud::keyring::FakeKeyring;

    fn session() -> Session {
        Session {
            access_token: "a".to_owned(),
            refresh_token: Some("r".to_owned()),
            expires_at: 9,
        }
    }

    fn store(dir: &TempDir, keyring: &Arc<FakeKeyring>) -> SessionStore {
        SessionStore::in_state_dir(dir.path(), Service::Fusion, keyring.clone())
    }

    fn bytes() -> Vec<u8> {
        serde_json::to_vec(&session()).unwrap()
    }

    #[test]
    fn nothing_saved_loads_as_no_session() {
        let dir = TempDir::new().unwrap();

        let store = store(&dir, &FakeKeyring::reachable());

        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn a_saved_session_goes_to_the_keyring_and_no_file_is_written() {
        let dir = TempDir::new().unwrap();
        let keyring = FakeKeyring::reachable();
        let store = store(&dir, &keyring);

        store.save(&session()).unwrap();

        assert_eq!(keyring.holds(Service::Fusion), Some(bytes()));
        assert!(!store.path.exists());
        assert_eq!(store.load().unwrap(), Some(session()));
    }

    #[test]
    fn forgetting_removes_the_session_from_the_keyring_and_the_file() {
        let dir = TempDir::new().unwrap();
        let keyring = FakeKeyring::reachable();
        let store = store(&dir, &keyring);
        store.save(&session()).unwrap();
        store.write_file(&bytes()).unwrap();

        store.forget().unwrap();
        store.forget().unwrap();

        assert_eq!(keyring.holds(Service::Fusion), None);
        assert!(!store.path.exists());
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn without_a_keyring_the_session_is_kept_in_the_file() {
        let dir = TempDir::new().unwrap();
        let store = store(&dir, &FakeKeyring::unreachable());

        store.save(&session()).unwrap();

        assert!(store.path.exists());
        assert_eq!(store.load().unwrap(), Some(session()));

        store.forget().unwrap();

        assert!(!store.path.exists());
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn a_file_from_before_the_keyring_moves_into_it_and_is_deleted() {
        let dir = TempDir::new().unwrap();
        let keyring = FakeKeyring::reachable();
        let store = store(&dir, &keyring);
        store.write_file(&bytes()).unwrap();

        assert_eq!(store.load().unwrap(), Some(session()));

        assert_eq!(keyring.holds(Service::Fusion), Some(bytes()));
        assert!(!store.path.exists());
        assert_eq!(store.load().unwrap(), Some(session()));
    }

    #[test]
    fn a_file_stays_while_the_keyring_cannot_take_it() {
        let dir = TempDir::new().unwrap();
        let keyring = FakeKeyring::unreachable();
        let store = store(&dir, &keyring);
        store.write_file(&bytes()).unwrap();

        assert_eq!(store.load().unwrap(), Some(session()));
        assert!(store.path.exists());

        keyring.reach(true);

        assert_eq!(store.load().unwrap(), Some(session()));
        assert!(!store.path.exists());
        assert_eq!(keyring.holds(Service::Fusion), Some(bytes()));
    }

    #[test]
    fn a_newer_file_wins_over_an_older_entry_in_the_keyring() {
        let dir = TempDir::new().unwrap();
        let keyring = FakeKeyring::reachable();
        let store = store(&dir, &keyring);
        store.save(&session()).unwrap();
        let renewed = Session {
            access_token: "b".to_owned(),
            ..session()
        };

        keyring.reach(false);
        store.save(&renewed).unwrap();
        keyring.reach(true);

        assert_eq!(store.load().unwrap(), Some(renewed.clone()));
        assert_eq!(
            keyring.holds(Service::Fusion),
            Some(serde_json::to_vec(&renewed).unwrap())
        );
    }

    #[test]
    fn saving_into_the_keyring_removes_a_file_left_behind() {
        let dir = TempDir::new().unwrap();
        let keyring = FakeKeyring::reachable();
        let store = store(&dir, &keyring);
        store.write_file(b"left behind").unwrap();

        store.save(&session()).unwrap();

        assert!(!store.path.exists());
    }

    #[test]
    fn a_damaged_file_is_dropped_rather_than_failing_every_start() {
        let dir = TempDir::new().unwrap();
        let store = store(&dir, &FakeKeyring::reachable());
        store.write_file(b"not json").unwrap();

        assert_eq!(store.load().unwrap(), None);
        assert!(!store.path.exists());
    }

    #[test]
    fn a_damaged_keyring_entry_is_dropped_rather_than_failing_every_start() {
        let dir = TempDir::new().unwrap();
        let keyring = FakeKeyring::reachable();
        keyring.put(Service::Fusion, b"not json");
        let store = store(&dir, &keyring);

        assert_eq!(store.load().unwrap(), None);
        assert_eq!(keyring.holds(Service::Fusion), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_session_kept_in_a_file_is_readable_by_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let store = store(&dir, &FakeKeyring::unreachable());

        store.save(&session()).unwrap();

        let mode = fs::metadata(&store.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }

    #[test]
    fn a_remembered_hub_loads_again_and_forgetting_removes_it() {
        let dir = TempDir::new().unwrap();
        let store =
            SessionStore::in_state_dir(dir.path(), Service::Fusion, FakeKeyring::unreachable());

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
        let store =
            SessionStore::in_state_dir(dir.path(), Service::Fusion, FakeKeyring::unreachable());
        store.remember_hub(&WorkspaceId::new("hub-2")).unwrap();

        fs::write(&store.hub_path, b"not json").unwrap();
        assert_eq!(store.remembered_hub(), None);

        fs::write(&store.hub_path, br#"{"workspace":""}"#).unwrap();
        assert_eq!(store.remembered_hub(), None);
    }

    #[test]
    fn each_service_remembers_its_own_hub() {
        let dir = TempDir::new().unwrap();
        let fusion =
            SessionStore::in_state_dir(dir.path(), Service::Fusion, FakeKeyring::unreachable());
        let onshape =
            SessionStore::in_state_dir(dir.path(), Service::Onshape, FakeKeyring::unreachable());

        fusion.remember_hub(&WorkspaceId::new("hub-2")).unwrap();

        assert_eq!(onshape.remembered_hub(), None);
    }

    fn remembered(hub: &str, folder: &str, name: &str) -> Remembered {
        Remembered {
            workspace: WorkspaceId::new(hub),
            folder: FolderRef {
                project: ProjectId::new("p1"),
                folder: FolderId::new(folder),
            },
            folder_name: name.to_owned(),
        }
    }

    #[test]
    fn destinations_of_every_hub_load_again_and_an_empty_list_removes_the_file() {
        let dir = TempDir::new().unwrap();
        let store =
            SessionStore::in_state_dir(dir.path(), Service::Fusion, FakeKeyring::unreachable());
        assert!(store.remembered_destinations().is_empty());

        let both = [
            remembered("hub-1", "f1", "Parts"),
            remembered("hub-2", "f2", "Frames"),
        ];
        store.remember_destinations(&both).unwrap();

        assert_eq!(store.remembered_destinations(), both);

        store.remember_destinations(&[]).unwrap();

        assert!(store.remembered_destinations().is_empty());
        assert!(!store.destination_path.exists());
    }

    #[test]
    fn a_damaged_destination_file_is_none_and_a_write_leaves_no_temporary_file() {
        let dir = TempDir::new().unwrap();
        let store =
            SessionStore::in_state_dir(dir.path(), Service::Fusion, FakeKeyring::unreachable());
        store
            .remember_destinations(&[remembered("hub-1", "f1", "Parts")])
            .unwrap();

        let names: Vec<_> = fs::read_dir(store.destination_path.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["fusion-destination.json".to_owned()]);

        fs::write(&store.destination_path, b"not json").unwrap();
        assert!(store.remembered_destinations().is_empty());

        fs::write(
            &store.destination_path,
            br#"{"destinations":[{"workspace":"","project":"p","folder":"f","folder_name":"x"}]}"#,
        )
        .unwrap();
        assert!(store.remembered_destinations().is_empty());
    }

    #[test]
    fn each_service_keeps_its_own_destinations_and_forgetting_removes_them() {
        let dir = TempDir::new().unwrap();
        let fusion =
            SessionStore::in_state_dir(dir.path(), Service::Fusion, FakeKeyring::unreachable());
        let onshape =
            SessionStore::in_state_dir(dir.path(), Service::Onshape, FakeKeyring::unreachable());
        fusion
            .remember_destinations(&[remembered("hub-1", "f1", "Parts")])
            .unwrap();

        assert!(onshape.remembered_destinations().is_empty());

        fusion.forget_destinations().unwrap();
        fusion.forget_destinations().unwrap();

        assert!(fusion.remembered_destinations().is_empty());
    }
}
