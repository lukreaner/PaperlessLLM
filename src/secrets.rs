use std::{path::Path, sync::Arc};

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, OsRng, rand_core::RngCore},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use zeroize::Zeroizing;

#[cfg(target_os = "macos")]
use sha2::{Digest, Sha256};

use crate::{AppError, AppResult, db::Database};

const MASTER_KEY_FILE: &str = ".master-key";
const KEYCHAIN_SERVICE: &str = "com.paperless-llm.credentials";

#[derive(Clone)]
pub struct SecretStore {
    database: Database,
    cipher: Arc<Aes256Gcm>,
    #[cfg(target_os = "macos")]
    keychain_prefix: Arc<String>,
}

impl SecretStore {
    pub async fn new(database: Database, data_dir: &Path) -> AppResult<Self> {
        let key = load_or_create_master_key(data_dir).await?;
        let cipher = Aes256Gcm::new_from_slice(key.as_slice())
            .map_err(|_| AppError::Internal("invalid credential encryption key".to_owned()))?;
        Ok(Self {
            database,
            cipher: Arc::new(cipher),
            #[cfg(target_os = "macos")]
            keychain_prefix: Arc::new(keychain_prefix(data_dir)),
        })
    }

    pub async fn set(&self, key: &str, value: &str) -> AppResult<()> {
        #[cfg(target_os = "macos")]
        if set_keychain(self.keychain_key(key), value.to_owned())
            .await
            .is_ok()
        {
            self.delete_encrypted(key).await?;
            return Ok(());
        }

        self.set_encrypted(key, value).await
    }

    pub async fn get(&self, key: &str) -> AppResult<Option<Zeroizing<String>>> {
        #[cfg(target_os = "macos")]
        if let Ok(Some(value)) = get_keychain(self.keychain_key(key)).await {
            return Ok(Some(Zeroizing::new(value)));
        }

        self.get_encrypted(key).await
    }

    pub async fn delete(&self, key: &str) -> AppResult<()> {
        #[cfg(target_os = "macos")]
        {
            let _ = delete_keychain(self.keychain_key(key)).await;
        }
        self.delete_encrypted(key).await
    }

    pub async fn exists(&self, key: &str) -> AppResult<bool> {
        Ok(self.get(key).await?.is_some())
    }

    async fn set_encrypted(&self, key: &str, value: &str) -> AppResult<()> {
        let mut nonce_bytes = [0_u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), value.as_bytes())
            .map_err(|_| AppError::Internal("credential encryption failed".to_owned()))?;

        sqlx::query(
            "INSERT INTO secret_values(key, nonce, ciphertext, updated_at) \
             VALUES(?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
             ON CONFLICT(key) DO UPDATE SET nonce=excluded.nonce, ciphertext=excluded.ciphertext, updated_at=excluded.updated_at",
        )
        .bind(key)
        .bind(nonce_bytes.to_vec())
        .bind(ciphertext)
        .execute(self.database.pool())
        .await?;
        Ok(())
    }

    async fn get_encrypted(&self, key: &str) -> AppResult<Option<Zeroizing<String>>> {
        let record: Option<(Vec<u8>, Vec<u8>)> =
            sqlx::query_as("SELECT nonce, ciphertext FROM secret_values WHERE key = ?")
                .bind(key)
                .fetch_optional(self.database.pool())
                .await?;

        let Some((nonce, ciphertext)) = record else {
            return Ok(None);
        };
        if nonce.len() != 12 {
            return Err(AppError::Internal(
                "stored credential nonce is invalid".to_owned(),
            ));
        }
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
            .map_err(|_| AppError::Internal("credential decryption failed".to_owned()))?;
        let value = String::from_utf8(plaintext)
            .map_err(|_| AppError::Internal("stored credential is not UTF-8".to_owned()))?;
        Ok(Some(Zeroizing::new(value)))
    }

    async fn delete_encrypted(&self, key: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM secret_values WHERE key = ?")
            .bind(key)
            .execute(self.database.pool())
            .await?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn keychain_key(&self, key: &str) -> String {
        format!("{}:{key}", self.keychain_prefix)
    }
}

#[cfg(target_os = "macos")]
fn keychain_prefix(data_dir: &Path) -> String {
    let digest = hex::encode(Sha256::digest(data_dir.to_string_lossy().as_bytes()));
    format!("instance:{}", &digest[..16])
}

async fn load_or_create_master_key(data_dir: &Path) -> AppResult<Zeroizing<Vec<u8>>> {
    if let Ok(raw) = std::env::var("PAPERLESSLLM_MASTER_KEY") {
        let bytes = decode_master_key(raw.trim())?;
        return Ok(Zeroizing::new(bytes));
    }

    let path = data_dir.join(MASTER_KEY_FILE);
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => {
            // Repair overly broad permissions on every startup, including for
            // key files restored from a backup.
            set_private_file_permissions(&path).await?;
            Ok(Zeroizing::new(decode_master_key(raw.trim())?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut bytes = vec![0_u8; 32];
            OsRng.fill_bytes(&mut bytes);
            let encoded = STANDARD.encode(&bytes);
            tokio::fs::write(&path, format!("{encoded}\n")).await?;
            set_private_file_permissions(&path).await?;
            Ok(Zeroizing::new(bytes))
        }
        Err(error) => Err(error.into()),
    }
}

fn decode_master_key(raw: &str) -> AppResult<Vec<u8>> {
    let bytes = STANDARD
        .decode(raw)
        .or_else(|_| hex::decode(raw))
        .map_err(|_| {
            AppError::Validation(
                "PAPERLESSLLM_MASTER_KEY must be 32 bytes encoded as base64 or hex".to_owned(),
            )
        })?;
    if bytes.len() != 32 {
        return Err(AppError::Validation(
            "PAPERLESSLLM_MASTER_KEY must decode to exactly 32 bytes".to_owned(),
        ));
    }
    Ok(bytes)
}

async fn set_private_file_permissions(path: &Path) -> AppResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn set_keychain(key: String, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &key).map_err(|e| e.to_string())?;
        entry.set_password(&value).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(target_os = "macos")]
async fn get_keychain(key: String) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &key).map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(target_os = "macos")]
async fn delete_keychain(key: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &key).map_err(|e| e.to_string())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_key_requires_32_bytes() {
        assert!(decode_master_key(&STANDARD.encode([7_u8; 32])).is_ok());
        assert!(decode_master_key(&STANDARD.encode([7_u8; 31])).is_err());
    }
}
