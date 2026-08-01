use std::{path::Path, sync::Arc};

#[cfg(not(target_os = "windows"))]
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, OsRng, rand_core::RngCore},
};
#[cfg(not(target_os = "windows"))]
use base64::{Engine as _, engine::general_purpose::STANDARD};
use zeroize::Zeroizing;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use sha2::{Digest, Sha256};

use crate::{AppError, AppResult, db::Database};

const MASTER_KEY_FILE: &str = ".master-key";
#[cfg(any(target_os = "macos", target_os = "windows"))]
const KEYCHAIN_SERVICE: &str = "com.paperless-llm.credentials";

#[derive(Clone)]
pub struct SecretStore {
    database: Database,
    #[cfg(not(target_os = "windows"))]
    cipher: Arc<Aes256Gcm>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    keychain_prefix: Arc<String>,
}

impl SecretStore {
    pub async fn new(database: Database, data_dir: &Path) -> AppResult<Self> {
        #[cfg(target_os = "windows")]
        harden_legacy_master_key_if_present(data_dir)?;
        #[cfg(not(target_os = "windows"))]
        let key = load_or_create_master_key(data_dir).await?;
        #[cfg(not(target_os = "windows"))]
        let cipher = Aes256Gcm::new_from_slice(key.as_slice())
            .map_err(|_| AppError::Internal("invalid credential encryption key".to_owned()))?;
        Ok(Self {
            database,
            #[cfg(not(target_os = "windows"))]
            cipher: Arc::new(cipher),
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            keychain_prefix: Arc::new(keychain_prefix(data_dir)),
        })
    }

    pub async fn set(&self, key: &str, value: &str) -> AppResult<()> {
        #[cfg(target_os = "windows")]
        {
            set_keychain(self.keychain_key(key), value.to_owned())
                .await
                .map_err(|_| {
                    AppError::Internal(
                        "Windows Credential Manager could not store the credential".to_owned(),
                    )
                })?;
            // Remove any encrypted row left by an experimental or restored
            // installation. Windows never reads this weaker fallback.
            self.delete_encrypted(key).await?;
            return Ok(());
        }

        #[cfg(target_os = "macos")]
        if set_keychain(self.keychain_key(key), value.to_owned())
            .await
            .is_ok()
        {
            self.delete_encrypted(key).await?;
            return Ok(());
        }

        #[cfg(not(target_os = "windows"))]
        return self.set_encrypted(key, value).await;
    }

    pub async fn get(&self, key: &str) -> AppResult<Option<Zeroizing<String>>> {
        #[cfg(target_os = "windows")]
        {
            return get_keychain(self.keychain_key(key))
                .await
                .map(|value| value.map(Zeroizing::new))
                .map_err(|_| {
                    AppError::Internal(
                        "Windows Credential Manager could not read the credential".to_owned(),
                    )
                });
        }

        #[cfg(target_os = "macos")]
        if let Ok(Some(value)) = get_keychain(self.keychain_key(key)).await {
            return Ok(Some(Zeroizing::new(value)));
        }

        #[cfg(not(target_os = "windows"))]
        return self.get_encrypted(key).await;
    }

    pub async fn delete(&self, key: &str) -> AppResult<()> {
        #[cfg(target_os = "windows")]
        delete_keychain(self.keychain_key(key)).await.map_err(|_| {
            AppError::Internal(
                "Windows Credential Manager could not delete the credential".to_owned(),
            )
        })?;
        #[cfg(target_os = "macos")]
        {
            let _ = delete_keychain(self.keychain_key(key)).await;
        }
        self.delete_encrypted(key).await
    }

    pub async fn exists(&self, key: &str) -> AppResult<bool> {
        Ok(self.get(key).await?.is_some())
    }

    #[cfg(not(target_os = "windows"))]
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

    #[cfg(not(target_os = "windows"))]
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

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn keychain_key(&self, key: &str) -> String {
        format!("{}:{key}", self.keychain_prefix)
    }
}

#[cfg(target_os = "windows")]
fn harden_legacy_master_key_if_present(data_dir: &Path) -> AppResult<()> {
    let path = data_dir.join(MASTER_KEY_FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => crate::private_fs::harden_private_file(&path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn keychain_prefix(data_dir: &Path) -> String {
    let mut digest = Sha256::new();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        digest.update(data_dir.as_os_str().as_bytes());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let canonical = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
        for unit in canonical.as_os_str().encode_wide() {
            digest.update(unit.to_le_bytes());
        }
    }
    let digest = hex::encode(digest.finalize());
    format!("instance:{}", &digest[..16])
}

#[cfg(not(target_os = "windows"))]
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

#[cfg(not(target_os = "windows"))]
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

#[cfg(not(target_os = "windows"))]
async fn set_private_file_permissions(path: &Path) -> AppResult<()> {
    crate::private_fs::harden_private_file(path)?;
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn set_keychain(key: String, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &key).map_err(|e| e.to_string())?;
        entry.set_password(&value).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    use super::*;

    #[test]
    fn master_key_requires_32_bytes() {
        assert!(decode_master_key(&STANDARD.encode([7_u8; 32])).is_ok());
        assert!(decode_master_key(&STANDARD.encode([7_u8; 31])).is_err());
    }
}
