use std::{fmt::Debug, sync::Arc};

use caditor_ai::ApiProvider;
use caditor_cloud::Service;

#[cfg(windows)]
mod credential_manager;
#[cfg(unix)]
mod secret_service;

#[cfg(test)]
pub use self::fake::{FakeAiKeyring, FakeKeyring};

#[derive(Debug, thiserror::Error)]
pub enum KeyringError {
    #[cfg(unix)]
    #[error("there is no desktop session bus ({0})")]
    NoSessionBus(zbus::Error),
    #[cfg(unix)]
    #[error("no Secret Service answers on the session bus ({0})")]
    NoSecretService(zbus::Error),
    #[cfg(unix)]
    #[error("the Secret Service has no default collection to keep secrets in")]
    NoDefaultCollection,
    #[cfg(unix)]
    #[error("the keyring stayed locked")]
    Locked,
    #[cfg(unix)]
    #[error("the keyring's unlock prompt was dismissed")]
    PromptDismissed,
    #[cfg(unix)]
    #[error("the Secret Service refused the request ({0})")]
    Refused(zbus::Error),
    #[cfg(windows)]
    #[error("the Credential Manager refused the request ({0})")]
    Credential(std::io::Error),
    #[cfg(windows)]
    #[error("the Credential Manager holds more pieces of a sign-in than caditor ever writes")]
    TooManyPieces,
    #[cfg(test)]
    #[error("the test keyring is unreachable")]
    Unreachable,
}

impl KeyringError {
    pub fn is_absent(&self) -> bool {
        match self {
            #[cfg(unix)]
            Self::NoSessionBus(_) | Self::NoSecretService(_) | Self::NoDefaultCollection => true,
            #[cfg(test)]
            Self::Unreachable => true,
            _ => false,
        }
    }
}

pub trait Keyring: Send + Sync + Debug {
    fn get(&self, service: Service) -> Result<Option<Vec<u8>>, KeyringError>;

    fn set(&self, service: Service, secret: &[u8]) -> Result<(), KeyringError>;

    fn delete(&self, service: Service) -> Result<(), KeyringError>;
}

pub trait AiKeyring: Send + Sync + Debug {
    fn get_key(&self, provider: ApiProvider) -> Result<Option<Vec<u8>>, KeyringError>;

    fn set_key(&self, provider: ApiProvider, secret: &[u8]) -> Result<(), KeyringError>;

    fn delete_key(&self, provider: ApiProvider) -> Result<(), KeyringError>;
}

pub fn system() -> Arc<dyn Keyring> {
    #[cfg(unix)]
    return Arc::new(secret_service::SecretService);
    #[cfg(windows)]
    return Arc::new(credential_manager::CredentialManager);
}

pub fn system_ai() -> Arc<dyn AiKeyring> {
    #[cfg(unix)]
    return Arc::new(secret_service::SecretService);
    #[cfg(windows)]
    return Arc::new(credential_manager::CredentialManager);
}

#[cfg(test)]
mod fake {
    use std::{
        collections::BTreeMap,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use caditor_ai::ApiProvider;
    use caditor_cloud::{Service, fake::Hold};
    use parking_lot::Mutex;

    use super::{AiKeyring, Keyring, KeyringError};

    #[derive(Debug, Default)]
    pub struct FakeKeyring {
        entries: Mutex<BTreeMap<Service, Vec<u8>>>,
        unreachable: AtomicBool,
        holding_sets: Mutex<Option<Arc<Hold>>>,
    }

    impl FakeKeyring {
        pub fn reachable() -> Arc<Self> {
            Arc::new(Self::default())
        }

        pub fn unreachable() -> Arc<Self> {
            let keyring = Self::default();
            keyring.unreachable.store(true, Ordering::Relaxed);
            Arc::new(keyring)
        }

        pub fn reach(&self, reachable: bool) {
            self.unreachable.store(!reachable, Ordering::Relaxed);
        }

        pub fn holds(&self, service: Service) -> Option<Vec<u8>> {
            self.entries.lock().get(&service).cloned()
        }

        pub fn hold_sets(&self, hold: Arc<Hold>) {
            *self.holding_sets.lock() = Some(hold);
        }

        pub fn put(&self, service: Service, secret: &[u8]) {
            self.entries.lock().insert(service, secret.to_vec());
        }

        fn check(&self) -> Result<(), KeyringError> {
            if self.unreachable.load(Ordering::Relaxed) {
                Err(KeyringError::Unreachable)
            } else {
                Ok(())
            }
        }
    }

    impl Keyring for FakeKeyring {
        fn get(&self, service: Service) -> Result<Option<Vec<u8>>, KeyringError> {
            self.check()?;
            Ok(self.entries.lock().get(&service).cloned())
        }

        fn set(&self, service: Service, secret: &[u8]) -> Result<(), KeyringError> {
            self.check()?;
            let held = self.holding_sets.lock().clone();
            if let Some(hold) = held {
                hold.stop_here();
            }
            self.entries.lock().insert(service, secret.to_vec());
            Ok(())
        }

        fn delete(&self, service: Service) -> Result<(), KeyringError> {
            self.check()?;
            self.entries.lock().remove(&service);
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    pub struct FakeAiKeyring {
        entries: Mutex<BTreeMap<ApiProvider, Vec<u8>>>,
        unreachable: AtomicBool,
    }

    impl FakeAiKeyring {
        pub fn reachable() -> Arc<Self> {
            Arc::new(Self::default())
        }

        pub fn unreachable() -> Arc<Self> {
            let keyring = Self::default();
            keyring.unreachable.store(true, Ordering::Relaxed);
            Arc::new(keyring)
        }

        fn check(&self) -> Result<(), KeyringError> {
            if self.unreachable.load(Ordering::Relaxed) {
                Err(KeyringError::Unreachable)
            } else {
                Ok(())
            }
        }
    }

    impl AiKeyring for FakeAiKeyring {
        fn get_key(&self, provider: ApiProvider) -> Result<Option<Vec<u8>>, KeyringError> {
            self.check()?;
            Ok(self.entries.lock().get(&provider).cloned())
        }

        fn set_key(&self, provider: ApiProvider, secret: &[u8]) -> Result<(), KeyringError> {
            self.check()?;
            self.entries.lock().insert(provider, secret.to_vec());
            Ok(())
        }

        fn delete_key(&self, provider: ApiProvider) -> Result<(), KeyringError> {
            self.check()?;
            self.entries.lock().remove(&provider);
            Ok(())
        }
    }
}
