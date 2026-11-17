mod clients;
mod grants;
mod metadata;
pub use clients::{ClientMetadata, Registration};
pub use grants::{AuthorizationRequest, Consent, TokenRequest, TokenResponse};

use crate::infrastructure::postgres::{Database, Error as DbError};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Access(#[from] crate::components::access::Error),
    #[error("invalid_request")]
    Request,
    #[error("unsupported_grant_type")]
    UnsupportedGrant,
    #[error("invalid_client")]
    Client,
    #[error("invalid_grant")]
    Grant,
    #[error("invalid_scope")]
    Scope,
    #[error("invalid_target")]
    Target,
    #[error("temporarily_unavailable")]
    Busy,
    #[error(transparent)]
    Database(#[from] DbError),
}

#[derive(Clone)]
pub struct OAuth {
    db: Database,
    access: crate::components::access::Access,
    hostname: String,
    registrations: Arc<Mutex<VecDeque<Instant>>>,
    metadata_requests: Arc<Semaphore>,
}
impl OAuth {
    pub fn with_access(mut self, access: crate::components::access::Access) -> Self {
        self.access = access;
        self
    }
    pub fn with_database(&self, db: Database) -> Self {
        Self { db, ..self.clone() }
    }

    pub fn new(db: Database, hostname: String) -> Self {
        Self {
            db,
            access: Default::default(),
            hostname,
            registrations: Arc::new(Mutex::new(VecDeque::new())),
            metadata_requests: Arc::new(Semaphore::new(4)),
        }
    }

    pub async fn sweep(&self) -> Result<(), Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT e.id FROM endpoints e WHERE EXISTS(SELECT 1 FROM oauth_grants g WHERE g.endpoint_id=e.id AND g.expires_at<=now()) ORDER BY e.id FOR KEY SHARE")
            .fetch_all(&mut *tx).await.map_err(DbError::from)?;
        sqlx::query!("SELECT id FROM oauth_grants WHERE expires_at<=now() ORDER BY id FOR UPDATE")
            .fetch_all(&mut *tx)
            .await
            .map_err(DbError::from)?;
        sqlx::query!("UPDATE access_tokens a SET revoked_at=COALESCE(a.revoked_at,now()) FROM oauth_grants g WHERE a.oauth_grant=g.id AND g.expires_at<=now()")
            .execute(&mut *tx).await.map_err(DbError::from)?;
        sqlx::query!("DELETE FROM oauth_grants WHERE expires_at<=now()")
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        sqlx::query!("DELETE FROM oauth_clients c WHERE updated_at<now()-interval '30 days' AND NOT EXISTS(SELECT 1 FROM oauth_grants g WHERE g.client_id=c.client_id)")
            .execute(&self.db.0).await.map_err(DbError::from)?;
        Ok(())
    }

    fn allow_registration(&self) -> Result<(), Error> {
        let mut attempts = self.registrations.lock().map_err(|_| Error::Busy)?;
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
        attempts.push_back(now);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    None,
    ClientSecretBasic,
    ClientSecretPost,
}
pub struct ClientAuthentication {
    pub client_id: String,
    pub method: AuthMethod,
    pub secret: Option<SecretString>,
}

fn random(prefix: &str) -> String {
    format!(
        "{prefix}{}",
        URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
    )
}
fn digest(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}

pub(super) async fn revoke_grant(tx: &mut sqlx::PgConnection, id: Uuid) -> Result<(), DbError> {
    sqlx::query!(
        "UPDATE oauth_grants SET revoked_at=COALESCE(revoked_at,now()) WHERE id=$1",
        id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE access_tokens SET revoked_at=COALESCE(revoked_at,now()) WHERE oauth_grant=$1",
        id
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(super) async fn revoke_access(db: &Database, id: Uuid) -> Result<bool, DbError> {
    let mut tx = db.0.begin().await?;
    // All grant operations lock endpoint, then grant, then access tokens.
    let grant = sqlx::query_scalar!("SELECT oauth_grant FROM access_tokens WHERE id=$1", id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
    if let Some(grant) = grant {
        sqlx::query!("SELECT e.id FROM endpoints e JOIN oauth_grants g ON g.endpoint_id=e.id WHERE g.id=$1 FOR KEY SHARE OF e",grant)
            .fetch_optional(&mut *tx).await?;
        sqlx::query!("SELECT id FROM oauth_grants WHERE id=$1 FOR UPDATE", grant)
            .fetch_optional(&mut *tx)
            .await?;
        let active = sqlx::query_scalar!(
            "SELECT revoked_at IS NULL AS \"active!\" FROM access_tokens WHERE id=$1",
            id
        )
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);
        revoke_grant(&mut tx, grant).await?;
        tx.commit().await?;
        return Ok(active);
    }
    let changed = sqlx::query!(
        "UPDATE access_tokens SET revoked_at=now() WHERE id=$1 AND revoked_at IS NULL",
        id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    tx.commit().await?;
    Ok(changed)
}
