use crate::infrastructure::postgres::{Database, Error as DbError};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("registration is limited to invited email addresses")]
    NotAllowed,
    #[error("email is already registered; sign in instead")]
    Exists,
    #[error("password must contain 12..1024 bytes")]
    Configuration,
    #[error("too many sign-in attempts; try again in one minute")]
    Busy,
    #[error("password verification failed")]
    Verification,
    #[error(transparent)]
    Database(#[from] DbError),
}

#[derive(Clone, Debug, serde::Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: uuid::Uuid,
    pub email: String,
    pub tenant_id: uuid::Uuid,
    pub admin: bool,
}

#[derive(Clone)]
pub struct Sessions {
    db: Database,
    workers: Arc<Semaphore>,
    attempts: Arc<Mutex<VecDeque<Instant>>>,
}

impl Sessions {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            workers: Arc::new(Semaphore::new(2)),
            attempts: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn first_user(&self) -> Result<bool, Error> {
        Ok(
            !sqlx::query_scalar!("SELECT EXISTS(SELECT 1 FROM users) AS \"exists!\"")
                .fetch_one(&self.db.0)
                .await
                .map_err(DbError::from)?,
        )
    }

    fn attempt(&self) -> Result<tokio::sync::OwnedSemaphorePermit, Error> {
        let mut attempts = self.attempts.lock().map_err(|_| Error::Busy)?;
        let now = Instant::now();
        while attempts
            .front()
            .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(60))
        {
            attempts.pop_front();
        }
        if attempts.len() >= 30 {
            return Err(Error::Busy);
        }
        let permit = self
            .workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        attempts.push_back(now);
        Ok(permit)
    }

    pub async fn register(
        &self,
        email: &str,
        password: SecretString,
    ) -> Result<SecretString, Error> {
        let permit = self.attempt()?;
        let email = allowed_email(email).ok_or(Error::NotAllowed)?;
        let hash = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            hash_password(password.expose_secret())
        })
        .await
        .map_err(|_| Error::Verification)??;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        // Serialize first-account adoption across gateway processes as well as concurrent requests.
        sqlx::query!("LOCK TABLE users IN SHARE ROW EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
        let first = !sqlx::query_scalar!("SELECT EXISTS(SELECT 1 FROM users) AS \"exists!\"")
            .fetch_one(&mut *tx)
            .await
            .map_err(DbError::from)?;
        let tenant = if first {
            uuid::Uuid::nil()
        } else {
            sqlx::query_scalar!("INSERT INTO tenants DEFAULT VALUES RETURNING id")
                .fetch_one(&mut *tx)
                .await
                .map_err(DbError::from)?
        };
        let user = sqlx::query_scalar!("INSERT INTO users(email,password_hash,tenant_id,admin) VALUES($1,$2,$3,$4) RETURNING id",email,hash,tenant,first)
            .fetch_one(&mut *tx).await.map_err(|e| match e {
                sqlx::Error::Database(ref e) if e.is_unique_violation() => Error::Exists,
                e => Error::Database(DbError::from(e)),
            })?;
        let token = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        sqlx::query!(
            "INSERT INTO sessions(token_hash,password_version,user_id) VALUES($1,$2,$3)",
            Sha256::digest(token.as_bytes()).to_vec(),
            Sha256::digest(hash.as_bytes()).to_vec(),
            user
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(token.into())
    }

    pub async fn login(
        &self,
        email: &str,
        password: SecretString,
    ) -> Result<Option<SecretString>, Error> {
        let permit = self.attempt()?;
        if password.expose_secret().len() > 1024 {
            return Ok(None);
        }
        let email = email.trim().to_ascii_lowercase();
        let row = sqlx::query!("SELECT id,password_hash FROM users WHERE email=$1", email)
            .fetch_optional(&self.db.0)
            .await
            .map_err(DbError::from)?;
        let hash = row.as_ref().map(|r| r.password_hash.clone());
        let valid = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            // Unknown addresses still perform password verification.
            let dummy = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXJhbmRvbXNhbHQ$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
            let hash = hash.as_deref().unwrap_or(dummy);
            validate_hash(hash).is_ok() && PasswordHash::new(hash).is_ok_and(|hash|
                Argon2::default().verify_password(password.expose_secret().as_bytes(), &hash).is_ok())
        }).await.map_err(|_| Error::Verification)?;
        let Some(row) = row.filter(|_| valid) else {
            return Ok(None);
        };
        let token = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        sqlx::query!(
            "INSERT INTO sessions(token_hash,password_version,user_id) VALUES($1,$2,$3)",
            Sha256::digest(token.as_bytes()).to_vec(),
            Sha256::digest(row.password_hash.as_bytes()).to_vec(),
            row.id
        )
        .execute(&self.db.0)
        .await
        .map_err(DbError::from)?;
        Ok(Some(token.into()))
    }

    pub async fn user(&self, token: &str) -> Result<Option<User>, Error> {
        if token.len() != 43
            || !token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Ok(None);
        }
        let hash = Sha256::digest(token.as_bytes()).to_vec();
        let row = sqlx::query!("SELECT u.id,u.email,u.tenant_id,u.admin,u.password_hash,s.password_version FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=$1 AND s.expires_at>now()",hash)
            .fetch_optional(&self.db.0).await.map_err(DbError::from)?;
        Ok(row
            .filter(|r| Sha256::digest(r.password_hash.as_bytes()).as_slice() == r.password_version)
            .map(|r| User {
                id: r.id,
                email: r.email,
                tenant_id: r.tenant_id,
                admin: r.admin,
            }))
    }
    pub async fn authenticate(&self, token: &str) -> Result<bool, Error> {
        Ok(self.user(token).await?.is_some())
    }
    pub async fn logout(&self, token: &str) -> Result<(), Error> {
        sqlx::query!(
            "DELETE FROM sessions WHERE token_hash=$1",
            Sha256::digest(token.as_bytes()).to_vec()
        )
        .execute(&self.db.0)
        .await
        .map_err(DbError::from)?;
        Ok(())
    }
    pub async fn sweep(&self) -> Result<(), Error> {
        sqlx::query!("DELETE FROM sessions WHERE expires_at<=now()")
            .execute(&self.db.0)
            .await
            .map_err(DbError::from)?;
        Ok(())
    }
}

fn allowed_email(email: &str) -> Option<String> {
    let email = email.trim().to_ascii_lowercase();
    let (local, domain) = email.split_once('@')?;
    if email.len() > 254
        || local.is_empty()
        || local.len() > 64
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&c))
    {
        return None;
    }
    (email == "ursus95@gmail.com" || domain == "olegpustovit.com").then_some(email)
}

fn validate_hash(hash: &str) -> Result<(), Error> {
    let parsed = PasswordHash::new(hash).map_err(|_| Error::Configuration)?;
    let params = argon2::Params::try_from(&parsed).map_err(|_| Error::Configuration)?;
    // Bound configured verification work as well as request concurrency.
    if !hash.starts_with("$argon2id$v=19$")
        || parsed.salt.is_none()
        || parsed.hash.as_ref().is_none_or(|output| output.len() < 32)
        || !(19456..=262144).contains(&params.m_cost())
        || !(2..=10).contains(&params.t_cost())
        || !(1..=8).contains(&params.p_cost())
    {
        return Err(Error::Configuration);
    }
    Ok(())
}

pub fn hash_password(password: &str) -> Result<String, Error> {
    if !(12..=1024).contains(&password.len()) {
        return Err(Error::Configuration);
    }
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|_| Error::Verification)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn administrator_hash_rejects_weak_or_unbounded_costs() {
        let hash = hash_password("fixture-browser-password").unwrap();
        assert!(validate_hash(&hash).is_ok());
        for invalid in [
            hash.replace("argon2id", "argon2i"),
            hash.replace("m=19456", "m=8"),
            hash.replace("m=19456", "m=4294967295"),
            hash.replace("t=2", "t=1"),
            hash.replace("t=2", "t=4294967295"),
            "not a hash".into(),
            "$argon2id$v=19$m=19456,t=2,p=1".into(),
        ] {
            assert!(validate_hash(&invalid).is_err());
        }
    }
}
