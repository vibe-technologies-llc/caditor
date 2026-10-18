use std::{
    collections::BTreeMap,
    future::poll_fn,
    pin::Pin,
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::Duration,
};

use caditor_cloud::Service;
use zbus::{
    Connection, Message, OwnedObjectPath, OwnedVariant, Proxy, Variant, block_on,
    export::futures_core::Stream, proxy::SignalStream,
};

use super::{Keyring, KeyringError};

const BUS_NAME: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";
const PROMPT_INTERFACE: &str = "org.freedesktop.Secret.Prompt";
const PLAIN: &str = "plain";
const DEFAULT_ALIAS: &str = "default";
const NO_OBJECT: &str = "/";
const LABEL_PROPERTY: &str = "org.freedesktop.Secret.Item.Label";
const ATTRIBUTES_PROPERTY: &str = "org.freedesktop.Secret.Item.Attributes";
const CONTENT_TYPE: &str = "application/json";
const APPLICATION: &str = "caditor";
const KIND: &str = "cloud-session";
const PROMPT_PATIENCE: Duration = Duration::from_secs(120);

type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

#[derive(Debug)]
pub struct SecretService;

impl Keyring for SecretService {
    fn get(&self, service: Service) -> Result<Option<Vec<u8>>, KeyringError> {
        block_on(async {
            let open = Open::new().await?;
            let Some(item) = open.find(service).await? else {
                return Ok(None);
            };
            let (_, _, value, _): Secret = open
                .proxy(&item, ITEM_INTERFACE)
                .await?
                .call("GetSecret", &(&open.session,))
                .await
                .map_err(KeyringError::Refused)?;
            Ok(Some(value))
        })
    }

    fn set(&self, service: Service, secret: &[u8]) -> Result<(), KeyringError> {
        block_on(async {
            let open = Open::new().await?;
            let label = format!("caditor {service} sign-in");
            let mut properties: BTreeMap<&str, Variant> = BTreeMap::new();
            properties.insert(LABEL_PROPERTY, Variant::from(label.as_str()));
            properties.insert(ATTRIBUTES_PROPERTY, Variant::from(attributes(service)));
            let wrapped = (
                &open.session,
                Vec::<u8>::new(),
                secret.to_vec(),
                CONTENT_TYPE,
            );
            let (_, prompt): (OwnedObjectPath, OwnedObjectPath) = open
                .proxy(&open.collection, COLLECTION_INTERFACE)
                .await?
                .call("CreateItem", &(properties, wrapped, true))
                .await
                .map_err(KeyringError::Refused)?;
            open.complete(&prompt).await
        })
    }

    fn delete(&self, service: Service) -> Result<(), KeyringError> {
        block_on(async {
            let open = Open::new().await?;
            let Some(item) = open.find(service).await? else {
                return Ok(());
            };
            let prompt: OwnedObjectPath = open
                .proxy(&item, ITEM_INTERFACE)
                .await?
                .call("Delete", &())
                .await
                .map_err(KeyringError::Refused)?;
            open.complete(&prompt).await
        })
    }
}

fn attributes(service: Service) -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("application", APPLICATION),
        ("kind", KIND),
        ("service", service.id()),
    ])
}

struct Open {
    connection: Connection,
    session: OwnedObjectPath,
    collection: OwnedObjectPath,
}

impl Open {
    async fn new() -> Result<Self, KeyringError> {
        let connection = Connection::session()
            .await
            .map_err(KeyringError::NoSessionBus)?;
        let service = Proxy::new(&connection, BUS_NAME, SERVICE_PATH, SERVICE_INTERFACE)
            .await
            .map_err(KeyringError::NoSecretService)?;
        let (_, session): (OwnedVariant, OwnedObjectPath) = service
            .call("OpenSession", &(PLAIN, Variant::from("")))
            .await
            .map_err(KeyringError::NoSecretService)?;
        let collection: OwnedObjectPath = service
            .call("ReadAlias", &(DEFAULT_ALIAS,))
            .await
            .map_err(KeyringError::Refused)?;
        if collection.as_str() == NO_OBJECT {
            return Err(KeyringError::NoDefaultCollection);
        }
        let open = Self {
            connection,
            session,
            collection,
        };
        open.unlock().await?;
        Ok(open)
    }

    async fn proxy(
        &self,
        path: &OwnedObjectPath,
        interface: &'static str,
    ) -> Result<Proxy<'static>, KeyringError> {
        Proxy::new(
            &self.connection,
            BUS_NAME,
            path.as_str().to_owned(),
            interface,
        )
        .await
        .map_err(KeyringError::Refused)
    }

    async fn is_locked(&self) -> Result<bool, KeyringError> {
        self.proxy(&self.collection, COLLECTION_INTERFACE)
            .await?
            .get_property("Locked")
            .await
            .map_err(KeyringError::Refused)
    }

    async fn unlock(&self) -> Result<(), KeyringError> {
        if !self.is_locked().await? {
            return Ok(());
        }
        let service = Proxy::new(&self.connection, BUS_NAME, SERVICE_PATH, SERVICE_INTERFACE)
            .await
            .map_err(KeyringError::Refused)?;
        let (_, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) = service
            .call("Unlock", &(vec![&self.collection],))
            .await
            .map_err(KeyringError::Refused)?;
        self.complete(&prompt).await?;
        if self.is_locked().await? {
            return Err(KeyringError::Locked);
        }
        Ok(())
    }

    async fn find(&self, service: Service) -> Result<Option<OwnedObjectPath>, KeyringError> {
        let found: Vec<OwnedObjectPath> = self
            .proxy(&self.collection, COLLECTION_INTERFACE)
            .await?
            .call("SearchItems", &(attributes(service),))
            .await
            .map_err(KeyringError::Refused)?;
        Ok(found.into_iter().next())
    }

    async fn complete(&self, prompt: &OwnedObjectPath) -> Result<(), KeyringError> {
        if prompt.as_str() == NO_OBJECT {
            return Ok(());
        }
        let proxy = self.proxy(prompt, PROMPT_INTERFACE).await?;
        let mut completed = proxy
            .receive_signal("Completed")
            .await
            .map_err(KeyringError::Refused)?;
        let _deadline = Deadline::start(proxy.clone());
        proxy
            .call::<_, _, ()>("Prompt", &("",))
            .await
            .map_err(KeyringError::Refused)?;
        let dismissed = next_message(&mut completed)
            .await
            .and_then(|message| message.body().deserialize::<(bool, OwnedVariant)>().ok())
            .is_none_or(|(dismissed, _)| dismissed);
        if dismissed {
            Err(KeyringError::PromptDismissed)
        } else {
            Ok(())
        }
    }
}

struct Deadline {
    _finished: mpsc::Sender<()>,
}

impl Deadline {
    fn start(prompt: Proxy<'static>) -> Self {
        let (finished, waiting) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("keyring prompt".to_owned())
            .spawn(move || {
                if waiting.recv_timeout(PROMPT_PATIENCE) == Err(RecvTimeoutError::Timeout) {
                    let dismissed = block_on(prompt.call::<_, _, ()>("Dismiss", &()));
                    if let Err(error) = dismissed {
                        log::debug!("the keyring prompt could not be dismissed: {error}");
                    }
                }
            });
        if let Err(error) = spawned {
            log::debug!("the keyring prompt has no deadline: {error}");
        }
        Self {
            _finished: finished,
        }
    }
}

async fn next_message(signals: &mut SignalStream<'_>) -> Option<Message> {
    poll_fn(|context| Pin::new(&mut *signals).poll_next(context)).await
}
