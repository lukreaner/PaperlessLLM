use std::{
    collections::{HashMap, VecDeque},
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, TimeDelta, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::{AppError, AppResult, db::Database, models::SessionInfo};

const LOGIN_WINDOW: Duration = Duration::from_secs(60);
const MAX_LOGIN_ATTEMPTS: usize = 5;

#[derive(Debug, Clone)]
pub struct CreatedSession {
    pub token: String,
    pub csrf_token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct AuthService {
    database: Database,
    login_attempts: Arc<Mutex<HashMap<IpAddr, VecDeque<Instant>>>>,
}

impl AuthService {
    pub fn new(database: Database) -> Self {
        Self {
            database,
            login_attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn has_admin_password(&self) -> AppResult<bool> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_auth WHERE singleton = 1")
            .fetch_one(self.database.pool())
            .await?;
        Ok(count == 1)
    }

    pub async fn set_admin_password(&self, password: &str) -> AppResult<()> {
        validate_password(password)?;
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| AppError::Internal("password hashing failed".to_owned()))?
            .to_string();

        sqlx::query(
            "INSERT INTO admin_auth(singleton, password_hash, updated_at) \
             VALUES(1, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
             ON CONFLICT(singleton) DO UPDATE SET password_hash=excluded.password_hash, updated_at=excluded.updated_at",
        )
        .bind(hash)
        .execute(self.database.pool())
        .await?;
        sqlx::query("DELETE FROM sessions")
            .execute(self.database.pool())
            .await?;
        Ok(())
    }

    pub async fn login(
        &self,
        ip: IpAddr,
        password: &str,
        session_hours: i64,
    ) -> AppResult<CreatedSession> {
        self.enforce_login_rate(ip).await?;
        let hash: Option<String> =
            sqlx::query_scalar("SELECT password_hash FROM admin_auth WHERE singleton = 1")
                .fetch_optional(self.database.pool())
                .await?;
        let hash = hash.ok_or_else(|| {
            AppError::Conflict("set an administrator password before enabling LAN mode".to_owned())
        })?;
        let parsed = PasswordHash::new(&hash)
            .map_err(|_| AppError::Internal("stored password hash is invalid".to_owned()))?;
        if Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_err()
        {
            return Err(AppError::Unauthorized);
        }

        let token = random_token(32);
        let csrf_token = random_token(24);
        let id_hash = hash_token(&token);
        let expires_at = Utc::now() + TimeDelta::hours(session_hours.clamp(1, 720));
        sqlx::query("INSERT INTO sessions(id_hash, csrf_token, expires_at) VALUES(?, ?, ?)")
            .bind(id_hash)
            .bind(&csrf_token)
            .bind(expires_at.to_rfc3339())
            .execute(self.database.pool())
            .await?;

        self.login_attempts.lock().await.remove(&ip);
        Ok(CreatedSession {
            token,
            csrf_token,
            expires_at,
        })
    }

    pub async fn authenticate(&self, token: &str) -> AppResult<Option<SessionInfo>> {
        let id_hash = hash_token(token);
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT csrf_token, expires_at FROM sessions WHERE id_hash = ? AND julianday(expires_at) > julianday('now')",
        )
        .bind(&id_hash)
        .fetch_optional(self.database.pool())
        .await?;

        let Some((csrf_token, expires_at)) = row else {
            return Ok(None);
        };
        let expires_at = DateTime::parse_from_rfc3339(&expires_at)
            .map_err(|_| AppError::Internal("stored session expiry is invalid".to_owned()))?
            .with_timezone(&Utc);
        sqlx::query(
            "UPDATE sessions SET last_seen_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id_hash=?",
        )
        .bind(&id_hash)
        .execute(self.database.pool())
        .await?;
        Ok(Some(SessionInfo {
            id_hash,
            csrf_token,
            expires_at,
        }))
    }

    pub async fn logout(&self, token: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM sessions WHERE id_hash = ?")
            .bind(hash_token(token))
            .execute(self.database.pool())
            .await?;
        Ok(())
    }

    async fn enforce_login_rate(&self, ip: IpAddr) -> AppResult<()> {
        let now = Instant::now();
        let mut all = self.login_attempts.lock().await;
        let attempts = all.entry(ip).or_default();
        while attempts
            .front()
            .is_some_and(|attempt| now.duration_since(*attempt) > LOGIN_WINDOW)
        {
            attempts.pop_front();
        }
        if attempts.len() >= MAX_LOGIN_ATTEMPTS {
            return Err(AppError::Forbidden(
                "too many login attempts; wait one minute".to_owned(),
            ));
        }
        attempts.push_back(now);
        Ok(())
    }
}

fn validate_password(password: &str) -> AppResult<()> {
    if password.len() < 12 {
        return Err(AppError::Validation(
            "administrator password must contain at least 12 characters".to_owned(),
        ));
    }
    if password.len() > 256 {
        return Err(AppError::Validation(
            "administrator password is too long".to_owned(),
        ));
    }
    Ok(())
}

fn random_token(bytes: usize) -> String {
    let mut value = vec![0_u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut value);
    URL_SAFE_NO_PAD.encode(value)
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_passwords() {
        assert!(validate_password("short").is_err());
        assert!(validate_password("a useful long password").is_ok());
    }

    #[test]
    fn token_hash_is_stable_and_not_plaintext() {
        assert_eq!(hash_token("token"), hash_token("token"));
        assert_ne!(hash_token("token"), "token");
    }
}
