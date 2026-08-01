use std::{path::Path, str::FromStr, time::Duration};

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

        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
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
