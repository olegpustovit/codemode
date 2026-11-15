use crate::infrastructure::postgres::{Database, Error as DbError};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use secrecy::{ExposeSecret, SecretBox, SecretString};
use std::collections::BTreeMap;
use uuid::Uuid;
mod apps;
mod oauth;
pub use apps::OAuthAppInput;
mod oauth_error;
pub use oauth_error::OAuthStage;
mod rest;
pub use oauth::OAuthCompletion;
pub use rest::HttpCredential;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("credential encryption is not configured")]
    Disabled,
    #[error("invalid credential or encryption key identifier")]
    Invalid,
    #[error("linked credential could not be authenticated; check the encryption key")]
    Authentication,
    #[error("source does not exist")]
    NotFound,
    #[error("upstream authorization failed at {stage}: {detail}")]
    OAuth { stage: OAuthStage, detail: String },
    #[error("invalid or expired authorization callback")]
    State,
    #[error("source or account changed; retry linking the account")]
    Changed,
    #[error("account authorization is busy; retry shortly")]
    Busy,
    #[error("provider does not support OAuth-app configuration")]
    Provider,
    #[error(transparent)]
    Registry(#[from] super::providers::Error),
    #[error(transparent)]
    Database(#[from] DbError),
}

#[derive(Clone)]
pub struct Accounts {
    db: Database,
    key: Option<std::sync::Arc<SecretBox<[u8; 32]>>>,
    key_id: String,
    refresh_locks: std::sync::Arc<[tokio::sync::Mutex<()>; 64]>,
    authorizations: std::sync::Arc<tokio::sync::Semaphore>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub source_id: Uuid,
    pub kind: String,
    pub key_id: String,
    pub expires_at: Option<String>,
}

impl Accounts {
    pub fn with_database(&self, db: Database) -> Self {
        Self { db, ..self.clone() }
    }

    pub fn new(db: Database, key: Option<[u8; 32]>, key_id: String) -> Result<Self, Error> {
        if key_id.is_empty()
            || key_id.len() > 64
            || !key_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            db,
            key: key.map(|k| std::sync::Arc::new(SecretBox::new(Box::new(k)))),
            key_id,
            refresh_locks: std::sync::Arc::new(std::array::from_fn(
                |_| tokio::sync::Mutex::new(()),
            )),
            authorizations: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
        })
    }

    pub async fn list(&self) -> Result<Vec<Account>, Error> {
        Ok(sqlx::query_as!(Account,r#"SELECT source_id,kind,key_id,to_char(expires_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS expires_at FROM accounts ORDER BY source_id"#).fetch_all(&self.db.0).await.map_err(DbError::from)?)
    }

    pub async fn link_bearer(&self, source: Uuid, credential: SecretString) -> Result<(), Error> {
        if !valid_bearer(credential.expose_secret()) {
            return Err(Error::Invalid);
        }
        self.store(source, "bearer", credential).await
    }

    pub async fn link_environment(
        &self,
        source: Uuid,
        environment: BTreeMap<String, SecretString>,
    ) -> Result<(), Error> {
        if environment.len() > 64
            || environment.iter().any(|(name, value)| {
                !environment_name(name) || value.expose_secret().contains('\0')
            })
        {
            return Err(Error::Invalid);
        }
        let values: BTreeMap<_, _> = environment
            .iter()
            .map(|(name, value)| (name, value.expose_secret()))
            .collect();
        let encoded = serde_json::to_string(&values).map_err(|_| Error::Invalid)?;
        if encoded.len() > 65536 {
            return Err(Error::Invalid);
        }
        self.store(source, "environment", encoded.into()).await
    }

    async fn store(&self, source: Uuid, kind: &str, credential: SecretString) -> Result<(), Error> {
        let key = self.key.as_ref().ok_or(Error::Disabled)?;
        let (ciphertext, nonce) = seal(
            key.expose_secret(),
            source,
            kind,
            &self.key_id,
            credential.expose_secret(),
        )?;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        let source_kind =
            sqlx::query_scalar!("SELECT kind FROM sources WHERE id=$1 FOR UPDATE", source)
                .fetch_optional(&mut *tx)
                .await
                .map_err(DbError::from)?
                .ok_or(Error::NotFound)?;
        if (kind == "environment") != (source_kind == "mcp_stdio")
            || (kind == "rest" && !matches!(source_kind.as_str(), "openapi" | "graphql"))
        {
            return Err(Error::Invalid);
        }
        sqlx::query!(
            "UPDATE sources SET revision=revision+1,status='pending',last_error=NULL WHERE id=$1",
            source
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        sqlx::query!("INSERT INTO accounts(source_id,kind,ciphertext,nonce,key_id) VALUES($1,$2,$3,$4,$5) ON CONFLICT(source_id) DO UPDATE SET kind=EXCLUDED.kind,ciphertext=EXCLUDED.ciphertext,nonce=EXCLUDED.nonce,key_id=EXCLUDED.key_id,expires_at=NULL,updated_at=now()",source,kind,ciphertext,nonce.as_slice(),self.key_id).execute(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }

    pub async fn unlink(&self, source: Uuid) -> Result<bool, Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!(
            "UPDATE sources SET revision=revision+1,status='pending',last_error=NULL WHERE id=$1",
            source
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        let removed = sqlx::query!("DELETE FROM accounts WHERE source_id=$1", source)
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?
            .rows_affected()
            > 0;
        tx.commit().await.map_err(DbError::from)?;
        Ok(removed)
    }

    pub(crate) async fn environment(
        &self,
        source: Uuid,
    ) -> Result<BTreeMap<String, SecretString>, Error> {
        let Some(credential) = self.load(source, "environment").await? else {
            return Ok(BTreeMap::new());
        };
        serde_json::from_str(credential.expose_secret()).map_err(|_| Error::Authentication)
    }

    async fn load(&self, source: Uuid, kind: &str) -> Result<Option<SecretString>, Error> {
        let row = sqlx::query!(
            "SELECT kind,ciphertext,nonce,key_id FROM accounts WHERE source_id=$1",
            source
        )
        .fetch_optional(&self.db.0)
        .await
        .map_err(DbError::from)?;
        let Some(row) = row else { return Ok(None) };
        if row.kind != kind || row.key_id != self.key_id {
            return Err(Error::Authentication);
        }
        let key = self.key.as_ref().ok_or(Error::Disabled)?;
        let nonce: [u8; 24] = row.nonce.try_into().map_err(|_| Error::Authentication)?;
        open(
            key.expose_secret(),
            source,
            kind,
            &row.key_id,
            &nonce,
            &row.ciphertext,
        )
        .map(Some)
    }
}

fn valid_bearer(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 65536
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-._~+/=".contains(&c))
}

pub(crate) fn environment_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && name
            .bytes()
            .enumerate()
            .all(|(i, c)| c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

fn seal(
    key: &[u8; 32],
    source: Uuid,
    kind: &str,
    key_id: &str,
    value: &str,
) -> Result<(Vec<u8>, [u8; 24]), Error> {
    let nonce = rand::random::<[u8; 24]>();
    let aad = format!("codemode-account-v1:{source}:{kind}:{key_id}");
    let ciphertext = XChaCha20Poly1305::new(key.into())
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: value.as_bytes(),
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| Error::Authentication)?;
    Ok((ciphertext, nonce))
}
fn open(
    key: &[u8; 32],
    source: Uuid,
    kind: &str,
    key_id: &str,
    nonce: &[u8; 24],
    ciphertext: &[u8],
) -> Result<SecretString, Error> {
    let aad = format!("codemode-account-v1:{source}:{kind}:{key_id}");
    let bytes = XChaCha20Poly1305::new(key.into())
        .decrypt(
            &XNonce::from(*nonce),
            Payload {
                msg: ciphertext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| Error::Authentication)?;
    String::from_utf8(bytes)
        .map(Into::into)
        .map_err(|_| Error::Authentication)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_randomized_authenticated_and_bound_to_source_kind_and_key_id() {
        let source = Uuid::new_v4();
        let (mut ciphertext, nonce) =
            seal(&[73; 32], source, "bearer", "test", "fixture-secret").unwrap();
        let (other_ciphertext, other_nonce) =
            seal(&[73; 32], source, "bearer", "test", "fixture-secret").unwrap();
        assert_ne!(nonce, other_nonce);
        assert_ne!(ciphertext, other_ciphertext);
        assert_eq!(
            open(&[73; 32], source, "bearer", "test", &nonce, &ciphertext)
                .unwrap()
                .expose_secret(),
            "fixture-secret"
        );
        assert!(
            open(
                &[73; 32],
                Uuid::new_v4(),
                "bearer",
                "test",
                &nonce,
                &ciphertext
            )
            .is_err()
        );
        assert!(open(&[74; 32], source, "bearer", "test", &nonce, &ciphertext).is_err());
        assert!(
            open(
                &[73; 32],
                source,
                "environment",
                "test",
                &nonce,
                &ciphertext
            )
            .is_err()
        );
        assert!(
            open(
                &[73; 32],
                source,
                "bearer",
                "other-key",
                &nonce,
                &ciphertext
            )
            .is_err()
        );
        ciphertext[0] ^= 1;
        assert!(open(&[73; 32], source, "bearer", "test", &nonce, &ciphertext).is_err());
    }
}
