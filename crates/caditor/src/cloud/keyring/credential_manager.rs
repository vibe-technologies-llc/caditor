use caditor_cloud::Service;
use caditor_windows::{
    CREDENTIAL_BLOB_LIMIT, delete_credential, read_credential, write_credential,
};

use super::{Keyring, KeyringError};

const USER: &str = "caditor";
const MOST_PIECES: usize = 64;

#[derive(Debug)]
pub struct CredentialManager;

fn target(service: Service, piece: usize) -> String {
    format!("caditor/{}/{piece}", service.id())
}

impl Keyring for CredentialManager {
    fn get(&self, service: Service) -> Result<Option<Vec<u8>>, KeyringError> {
        let mut secret = Vec::new();
        for piece in 0..MOST_PIECES {
            match read_credential(&target(service, piece)).map_err(KeyringError::Credential)? {
                Some(bytes) => secret.extend(bytes),
                None if piece == 0 => return Ok(None),
                None => return Ok(Some(secret)),
            }
        }
        Err(KeyringError::TooManyPieces)
    }

    fn set(&self, service: Service, secret: &[u8]) -> Result<(), KeyringError> {
        let pieces: Vec<&[u8]> = if secret.is_empty() {
            vec![secret]
        } else {
            secret.chunks(CREDENTIAL_BLOB_LIMIT).collect()
        };
        if pieces.len() > MOST_PIECES {
            return Err(KeyringError::TooManyPieces);
        }
        for (piece, bytes) in pieces.iter().enumerate() {
            write_credential(&target(service, piece), USER, bytes)
                .map_err(KeyringError::Credential)?;
        }
        remove_from(service, pieces.len())
    }

    fn delete(&self, service: Service) -> Result<(), KeyringError> {
        remove_from(service, 0)
    }
}

fn remove_from(service: Service, first: usize) -> Result<(), KeyringError> {
    for piece in first..MOST_PIECES {
        let deleted =
            delete_credential(&target(service, piece)).map_err(KeyringError::Credential)?;
        if !deleted {
            break;
        }
    }
    Ok(())
}
