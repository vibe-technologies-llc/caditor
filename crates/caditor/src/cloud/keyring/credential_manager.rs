use caditor_ai::ApiProvider;
use caditor_cloud::Service;
use caditor_windows::{
    CREDENTIAL_BLOB_LIMIT, delete_credential, read_credential, write_credential,
};

use super::{AiKeyring, Keyring, KeyringError};

const USER: &str = "caditor";
const MOST_PIECES: usize = 64;

#[derive(Debug)]
pub struct CredentialManager;

fn target(service: Service, piece: usize) -> String {
    format!("caditor/{}/{piece}", service.id())
}

fn ai_target(provider: ApiProvider, piece: usize) -> String {
    format!("caditor/ai/{}/{piece}", provider.id())
}

impl Keyring for CredentialManager {
    fn get(&self, service: Service) -> Result<Option<Vec<u8>>, KeyringError> {
        read_pieces(|piece| target(service, piece))
    }

    fn set(&self, service: Service, secret: &[u8]) -> Result<(), KeyringError> {
        write_pieces(|piece| target(service, piece), secret)
    }

    fn delete(&self, service: Service) -> Result<(), KeyringError> {
        remove_from(|piece| target(service, piece), 0)
    }
}

impl AiKeyring for CredentialManager {
    fn get_key(&self, provider: ApiProvider) -> Result<Option<Vec<u8>>, KeyringError> {
        read_pieces(|piece| ai_target(provider, piece))
    }

    fn set_key(&self, provider: ApiProvider, secret: &[u8]) -> Result<(), KeyringError> {
        write_pieces(|piece| ai_target(provider, piece), secret)
    }

    fn delete_key(&self, provider: ApiProvider) -> Result<(), KeyringError> {
        remove_from(|piece| ai_target(provider, piece), 0)
    }
}

fn read_pieces(make_target: impl Fn(usize) -> String) -> Result<Option<Vec<u8>>, KeyringError> {
    let mut secret = Vec::new();
    for piece in 0..MOST_PIECES {
        match read_credential(&make_target(piece)).map_err(KeyringError::Credential)? {
            Some(bytes) => secret.extend(bytes),
            None if piece == 0 => return Ok(None),
            None => return Ok(Some(secret)),
        }
    }
    Err(KeyringError::TooManyPieces)
}

fn write_pieces(make_target: impl Fn(usize) -> String, secret: &[u8]) -> Result<(), KeyringError> {
    let pieces: Vec<&[u8]> = if secret.is_empty() {
        vec![secret]
    } else {
        secret.chunks(CREDENTIAL_BLOB_LIMIT).collect()
    };
    if pieces.len() > MOST_PIECES {
        return Err(KeyringError::TooManyPieces);
    }
    for (piece, bytes) in pieces.iter().enumerate() {
        write_credential(&make_target(piece), USER, bytes).map_err(KeyringError::Credential)?;
    }
    remove_from(make_target, pieces.len())
}

fn remove_from(make_target: impl Fn(usize) -> String, first: usize) -> Result<(), KeyringError> {
    for piece in first..MOST_PIECES {
        let deleted = delete_credential(&make_target(piece)).map_err(KeyringError::Credential)?;
        if !deleted {
            break;
        }
    }
    Ok(())
}
