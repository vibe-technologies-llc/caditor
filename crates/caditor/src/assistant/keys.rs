use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use caditor_ai::{ApiKey, ApiProvider};

use crate::{
    assistant::state::KeySource,
    cloud::{
        keyring::{AiKeyring, KeyringError},
        private,
    },
};

const FOLDER: &str = "assistant";

#[derive(Debug, thiserror::Error)]
pub enum KeyStoreError {
    #[error("the API key file could not be written ({0})")]
    File(io::Error),
    #[error("the system keyring refused to remove the API key ({0})")]
    Keyring(KeyringError),
}

#[derive(Clone)]
pub struct KeyStore {
    folder: Option<PathBuf>,
    keyring: Arc<dyn AiKeyring>,
    read_env: bool,
}

impl KeyStore {
    pub fn new(state_dir: Option<&Path>, keyring: Arc<dyn AiKeyring>) -> Self {
        Self {
            folder: state_dir.map(|dir| dir.join(FOLDER)),
            keyring,
            read_env: true,
        }
    }

    #[cfg(test)]
    pub fn without_env(mut self) -> Self {
        self.read_env = false;
        self
    }

    pub fn load_all(&self) -> BTreeMap<ApiProvider, (ApiKey, KeySource)> {
        let mut loaded = BTreeMap::new();

        for provider in ApiProvider::ALL {
            loaded.insert(provider, self.load(provider));
        }
        loaded
    }

    pub fn load(&self, provider: ApiProvider) -> (ApiKey, KeySource) {
        if let Some(folder) = &self.folder {
            let _ = private::restrict(folder);
        }
        if let Some(file_key) = self.read_file(provider) {
            if self
                .keyring
                .set_key(provider, file_key.as_str().as_bytes())
                .is_ok()
            {
                let _ = self.remove_file(provider);

                return (file_key, KeySource::Keyring);
            }
            return (file_key, KeySource::PrivateFile);
        }
        match self.keyring.get_key(provider) {
            Ok(Some(bytes)) => {
                if let Ok(text) = String::from_utf8(bytes) {
                    let key = ApiKey::new(text);
                    if !key.is_empty() {
                        return (key, KeySource::Keyring);
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                if !error.is_absent() {
                    log::debug!("could not read {provider} API key from keyring: {error}");
                }
            }
        }
        if self.read_env
            && let Some(var) = provider.env_var()
            && let Ok(raw) = env::var(var)
        {
            let key = ApiKey::new(raw);
            if !key.is_empty() {
                return (key, KeySource::Environment(var));
            }
        }
        (ApiKey::default(), KeySource::Missing)
    }

    pub fn save(&self, provider: ApiProvider, key: &ApiKey) -> Result<KeySource, KeyStoreError> {
        if key.is_empty() {
            self.clear(provider)?;
            return Ok(self.load(provider).1);
        }
        match self.keyring.set_key(provider, key.as_str().as_bytes()) {
            Ok(()) => {
                let _ = self.remove_file(provider);
                Ok(KeySource::Keyring)
            }
            Err(error) => {
                log::debug!("keyring unavailable for {provider} key ({error}); using private file");
                self.write_file(provider, key.as_str().as_bytes())
                    .map_err(KeyStoreError::File)?;
                Ok(KeySource::PrivateFile)
            }
        }
    }

    pub fn clear(&self, provider: ApiProvider) -> Result<(), KeyStoreError> {
        let file = self.remove_file(provider).map_err(KeyStoreError::File);
        let keyring = match self.keyring.delete_key(provider) {
            Err(error) if !error.is_absent() => Err(KeyStoreError::Keyring(error)),
            _ => Ok(()),
        };
        file.and(keyring)
    }

    fn path_for(&self, provider: ApiProvider) -> Option<PathBuf> {
        self.folder
            .as_ref()
            .map(|folder| folder.join(format!("{}-key.txt", provider.id())))
    }

    fn read_file(&self, provider: ApiProvider) -> Option<ApiKey> {
        let path = self.path_for(provider)?;
        let text = fs::read_to_string(path).ok()?;
        let key = ApiKey::new(text);
        (!key.is_empty()).then_some(key)
    }

    fn write_file(&self, provider: ApiProvider, bytes: &[u8]) -> io::Result<()> {
        let Some(folder) = &self.folder else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no state directory is available",
            ));
        };

        private::create_dir_all(folder)?;

        let path = folder.join(format!("{}-key.txt", provider.id()));
        caditor_file::write_privately(&path, bytes)
    }

    fn remove_file(&self, provider: ApiProvider) -> io::Result<()> {
        let Some(path) = self.path_for(provider) else {
            return Ok(());
        };
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::cloud::keyring::FakeAiKeyring;

    #[test]
    fn key_store_uses_keyring_and_falls_back_to_private_file_when_unreachable() {
        let dir = TempDir::new().unwrap();
        let keyring = FakeAiKeyring::reachable();

        let store = KeyStore::new(Some(dir.path()), keyring).without_env();

        let source = store
            .save(ApiProvider::Claude, &ApiKey::new("sk-ant-123"))
            .unwrap();

        assert_eq!(source, KeySource::Keyring);

        let (loaded, found_source) = store.load(ApiProvider::Claude);

        assert_eq!(loaded.as_str(), "sk-ant-123");
        assert_eq!(found_source, KeySource::Keyring);

        store.clear(ApiProvider::Claude).unwrap();

        assert_eq!(store.load(ApiProvider::Claude).1, KeySource::Missing);

        let unreachable = FakeAiKeyring::unreachable();
        let fallback = KeyStore::new(Some(dir.path()), unreachable).without_env();
        let file_source = fallback
            .save(ApiProvider::OpenAi, &ApiKey::new("sk-openai-456"))
            .unwrap();

        assert_eq!(file_source, KeySource::PrivateFile);

        let (loaded, source) = fallback.load(ApiProvider::OpenAi);

        assert_eq!(loaded.as_str(), "sk-openai-456");
        assert_eq!(source, KeySource::PrivateFile);
    }
}
