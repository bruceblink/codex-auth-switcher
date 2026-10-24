//! Versioned AES-GCM records. Purpose and header are authenticated; keys are never logged.

use crate::{Error, Paths, Result, files};
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
};
use zeroize::Zeroizing;

const HEADER: &[u8] = b"CAS2\0\x01";

pub struct Cipher {
    key: Zeroizing<[u8; 32]>,
}
impl Cipher {
    pub fn open(paths: &Paths, initialize: bool) -> Result<Self> {
        let raw = files::read(&paths.key(), 32)?;
        let key = match raw {
            Some(raw) => Zeroizing::new(
                raw.as_slice()
                    .try_into()
                    .map_err(|_| Error("Invalid local encryption key."))?,
            ),
            None if initialize && !paths.vault().exists() && !paths.journal().exists() => {
                let generated = Aes256Gcm::generate_key(&mut OsRng);
                let key = Zeroizing::new(<[u8; 32]>::from(generated));
                files::create(&paths.key(), key.as_ref())?;
                key
            }
            None => {
                return Err(Error(
                    "Encryption key is missing. Restore vault.key; existing ciphertext was not changed.",
                ));
            }
        };
        Ok(Self { key })
    }

    pub fn encrypt(&self, purpose: &str, plain: &[u8]) -> Result<Vec<u8>> {
        if plain.len() > files::STATE_LIMIT - 64 {
            return Err(Error("Encrypted state exceeds the size limit."));
        }
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref())
            .map_err(|_| Error("Invalid encryption key."))?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let aad = [HEADER, purpose.as_bytes()].concat();
        let encrypted = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plain,
                    aad: &aad,
                },
            )
            .map_err(|_| Error("State encryption failed."))?;
        Ok([HEADER, nonce.as_slice(), &encrypted].concat())
    }

    pub fn decrypt(&self, purpose: &str, encrypted: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        if !encrypted.starts_with(HEADER)
            || encrypted.len() < HEADER.len() + 12 + 16
            || encrypted.len() > files::STATE_LIMIT
        {
            return Err(Error("Invalid encrypted state format or size."));
        }
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref())
            .map_err(|_| Error("Invalid encryption key."))?;
        let nonce = Nonce::from_slice(&encrypted[HEADER.len()..HEADER.len() + 12]);
        let aad = [HEADER, purpose.as_bytes()].concat();
        cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &encrypted[HEADER.len() + 12..],
                    aad: &aad,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| {
                Error("Cannot authenticate encrypted state. Check the key and ciphertext.")
            })
    }
}
