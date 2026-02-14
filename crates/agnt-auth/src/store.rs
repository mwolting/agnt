use std::collections::HashMap;
use std::sync::Arc;

use agnt_db::Store;
use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use parking_lot::Mutex;
use rand::random;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use serde::{Deserialize, Serialize};

use crate::error::Error;

const ENCRYPTION_KEY_ACCOUNT: &str = "provider_credentials_key_v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StoredCredential {
    ApiKey {
        api_key: String,
    },
    OAuthPkce {
        access_token: String,
        refresh_token: String,
        expires_at_ms: u64,
        #[serde(default)]
        metadata: HashMap<String, String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CredentialEncryptionMethod {
    None,
    #[serde(rename = "keyring_aes_256_gcm_v1")]
    KeyringAes256GcmV1,
}

#[derive(Debug, Serialize, Deserialize)]
struct CredentialEnvelope {
    method: CredentialEncryptionMethod,
    payload: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
}

pub struct CredentialStore {
    service: String,
    store: Arc<Mutex<Store>>,
}

impl CredentialStore {
    pub fn new(service: impl Into<String>, store: Arc<Mutex<Store>>) -> Self {
        Self {
            service: service.into(),
            store,
        }
    }

    pub fn load(&self, provider_id: &str) -> Result<Option<StoredCredential>, Error> {
        let raw = {
            let mut store = self.store.lock();
            store.provider_credentials().get(provider_id)?
        };

        match raw {
            Some(raw) => Ok(Some(self.decode_credential(provider_id, &raw)?)),
            None => Ok(None),
        }
    }

    pub fn save(&self, provider_id: &str, credential: &StoredCredential) -> Result<(), Error> {
        let encoded = self.encode_credential(provider_id, credential)?;
        let mut store = self.store.lock();
        store
            .provider_credentials()
            .upsert(provider_id, &encoded)
            .map_err(Error::from)
    }

    fn encode_credential(
        &self,
        provider_id: &str,
        credential: &StoredCredential,
    ) -> Result<String, Error> {
        let credential_json = serde_json::to_string(credential)?;

        let envelope = match default_write_method() {
            CredentialEncryptionMethod::None => CredentialEnvelope {
                method: CredentialEncryptionMethod::None,
                payload: credential_json,
                nonce: None,
            },
            CredentialEncryptionMethod::KeyringAes256GcmV1 => {
                let key = self.load_or_create_encryption_key()?;
                let nonce: [u8; 12] = random();
                let ciphertext =
                    encrypt_credential(&key, nonce, provider_id, credential_json.as_bytes())?;

                CredentialEnvelope {
                    method: CredentialEncryptionMethod::KeyringAes256GcmV1,
                    payload: STANDARD_NO_PAD.encode(ciphertext),
                    nonce: Some(STANDARD_NO_PAD.encode(nonce)),
                }
            }
        };

        Ok(serde_json::to_string(&envelope)?)
    }

    fn decode_credential(&self, provider_id: &str, raw: &str) -> Result<StoredCredential, Error> {
        let envelope = match serde_json::from_str::<CredentialEnvelope>(raw) {
            Ok(envelope) => envelope,
            Err(_) => {
                // Backward compatibility for any plaintext credential blobs.
                return Ok(serde_json::from_str(raw)?);
            }
        };

        match envelope.method {
            CredentialEncryptionMethod::None => Ok(serde_json::from_str(&envelope.payload)?),
            CredentialEncryptionMethod::KeyringAes256GcmV1 => {
                let nonce_raw = envelope
                    .nonce
                    .ok_or_else(|| Error::Other("missing credential nonce".to_string()))?;
                let nonce_bytes = decode_nonce(&nonce_raw)?;
                let ciphertext = STANDARD_NO_PAD.decode(&envelope.payload).map_err(|err| {
                    Error::Other(format!("invalid credential ciphertext encoding: {err}"))
                })?;
                let key = self.load_encryption_key()?;
                let plaintext = decrypt_credential(&key, nonce_bytes, provider_id, ciphertext)?;
                let plaintext = String::from_utf8(plaintext)
                    .map_err(|err| Error::Other(format!("invalid credential plaintext: {err}")))?;
                Ok(serde_json::from_str(&plaintext)?)
            }
        }
    }

    fn load_or_create_encryption_key(&self) -> Result<[u8; 32], Error> {
        let entry = keyring::Entry::new(&self.service, ENCRYPTION_KEY_ACCOUNT)?;

        match entry.get_password() {
            Ok(encoded) => decode_key(&encoded),
            Err(keyring::Error::NoEntry) => {
                let key: [u8; 32] = random();
                entry.set_password(&STANDARD_NO_PAD.encode(key))?;
                Ok(key)
            }
            Err(err) => Err(err.into()),
        }
    }

    fn load_encryption_key(&self) -> Result<[u8; 32], Error> {
        let entry = keyring::Entry::new(&self.service, ENCRYPTION_KEY_ACCOUNT)?;
        let encoded = entry.get_password()?;
        decode_key(&encoded)
    }
}

fn default_write_method() -> CredentialEncryptionMethod {
    #[cfg(debug_assertions)]
    {
        CredentialEncryptionMethod::None
    }

    #[cfg(not(debug_assertions))]
    {
        CredentialEncryptionMethod::KeyringAes256GcmV1
    }
}

fn decode_key(encoded: &str) -> Result<[u8; 32], Error> {
    let bytes = STANDARD_NO_PAD
        .decode(encoded)
        .map_err(|err| Error::Other(format!("invalid encryption key encoding: {err}")))?;
    bytes
        .try_into()
        .map_err(|_| Error::Other("invalid encryption key length; expected 32 bytes".to_string()))
}

fn decode_nonce(encoded: &str) -> Result<[u8; 12], Error> {
    let bytes = STANDARD_NO_PAD
        .decode(encoded)
        .map_err(|err| Error::Other(format!("invalid credential nonce encoding: {err}")))?;
    bytes
        .try_into()
        .map_err(|_| Error::Other("invalid credential nonce length; expected 12 bytes".to_string()))
}

fn encrypt_credential(
    key: &[u8; 32],
    nonce: [u8; 12],
    provider_id: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>, Error> {
    let unbound = UnboundKey::new(&AES_256_GCM, key)
        .map_err(|_| Error::Other("invalid encryption key material".to_string()))?;
    let key = LessSafeKey::new(unbound);

    let mut in_out = plaintext.to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(provider_id.as_bytes()),
        &mut in_out,
    )
    .map_err(|_| Error::Other("failed to encrypt credential".to_string()))?;

    Ok(in_out)
}

fn decrypt_credential(
    key: &[u8; 32],
    nonce: [u8; 12],
    provider_id: &str,
    mut ciphertext: Vec<u8>,
) -> Result<Vec<u8>, Error> {
    let unbound = UnboundKey::new(&AES_256_GCM, key)
        .map_err(|_| Error::Other("invalid encryption key material".to_string()))?;
    let key = LessSafeKey::new(unbound);

    let plaintext = key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(provider_id.as_bytes()),
            &mut ciphertext,
        )
        .map_err(|_| Error::Other("failed to decrypt credential".to_string()))?;

    Ok(plaintext.to_vec())
}
