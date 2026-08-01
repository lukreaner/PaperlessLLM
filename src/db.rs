use std::{path::Path, time::Duration};

use serde::{Serialize, de::DeserializeOwned};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::{AppError, AppResult};

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn connect(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        prepare_private_database_file(path)?;

        // Passing the path directly avoids URL parsing ambiguities for Windows
        // drive letters, backslashes, spaces, and non-Unicode filenames.
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(10));

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(15))
            .connect_with(options)
            .await?;

        sqlx::migrate!("./migrations").run(&pool).await?;
        sqlx::query("PRAGMA optimize").execute(&pool).await?;
        harden_sqlite_file_permissions(path)?;

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn get_setting<T: DeserializeOwned>(&self, key: &str) -> AppResult<Option<T>> {
        let raw: Option<String> =
            sqlx::query_scalar("SELECT value_json FROM app_settings WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;

        raw.map(|value| serde_json::from_str(&value).map_err(AppError::from))
            .transpose()
    }

    pub async fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> AppResult<()> {
        let value_json = serde_json::to_string(value)?;
        sqlx::query(
            "INSERT INTO app_settings(key, value_json, updated_at) VALUES(?, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
             ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, updated_at=excluded.updated_at",
        )
        .bind(key)
        .bind(value_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn is_configured(&self) -> AppResult<bool> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM processing_profiles WHERE enabled = 1")
                .fetch_one(&self.pool)
                .await?;
        Ok(count > 0)
    }

    pub async fn cleanup_expired_sessions(&self) -> AppResult<()> {
        sqlx::query("DELETE FROM sessions WHERE julianday(expires_at) <= julianday('now')")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn prepare_private_database_file(path: &Path) -> AppResult<()> {
    // Creating the database ourselves applies private Unix mode/open flags or
    // the inherited private Windows ACL from its first filesystem object.
    // Re-applying permissions also repairs databases restored from backups.
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    crate::private_fs::configure_private_open(&mut options, 0o600);
    let _file = options.open(path)?;
    crate::private_fs::harden_private_file(path)?;
    Ok(())
}

fn harden_sqlite_file_permissions(path: &Path) -> AppResult<()> {
    use std::{ffi::OsString, path::PathBuf};

    for suffix in ["", "-wal", "-shm"] {
        let mut candidate: OsString = path.as_os_str().to_owned();
        candidate.push(suffix);
        let candidate = PathBuf::from(candidate);
        match crate::private_fs::harden_private_file(&candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::Database;

    #[tokio::test]
    async fn connect_repairs_database_and_sidecar_permissions() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("paperless-llm.db");
        std::fs::write(&path, []).expect("create database placeholder");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("make database deliberately too broad");

        let database = Database::connect(&path).await.expect("database");

        for candidate in [
            path.clone(),
            path.with_file_name("paperless-llm.db-wal"),
            path.with_file_name("paperless-llm.db-shm"),
        ] {
            if candidate.exists() {
                let mode = std::fs::metadata(&candidate)
                    .expect("database metadata")
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, 0o600, "{} was not private", candidate.display());
            }
        }

        database.pool().close().await;
    }
}
