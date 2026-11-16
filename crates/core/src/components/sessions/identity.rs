use super::{Sessions, User, allowed_email, normalize_email};
use crate::infrastructure::postgres::Error as DbError;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use secrecy::SecretString;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("invalid verified identity")]
    InvalidClaims,
    #[error("identity requires an existing local account")]
    AccountRequired,
    #[error("registration is limited to invited email addresses")]
    NotAllowed,
    #[error("local account is bound to another identity")]
    Conflict,
    #[error(transparent)]
    Database(#[from] DbError),
}

impl Sessions {
    /// Accept only SDK-verified claims. Roles and tenant membership remain local.
    pub async fn link_identity(&self, subject: &str, email: &str) -> Result<User, IdentityError> {
        self.resolve_identity_account(subject, email, false).await
    }

    /// Accept only SDK-verified claims; new accounts retain the local signup allowlist.
    pub async fn resolve_identity(
        &self,
        subject: &str,
        email: &str,
    ) -> Result<User, IdentityError> {
        self.resolve_identity_account(subject, email, true).await
    }

    async fn resolve_identity_account(
        &self,
        subject: &str,
        email: &str,
        provision: bool,
    ) -> Result<User, IdentityError> {
        if subject.is_empty() || subject.len() > 255 || subject.contains('\0') {
            return Err(IdentityError::InvalidClaims);
        }
        let email = normalize_email(email).ok_or(IdentityError::InvalidClaims)?;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        if provision {
            // Serialize signup with legacy registration without adopting its first-user tenant.
            sqlx::query!("LOCK TABLE users IN SHARE ROW EXCLUSIVE MODE")
                .execute(&mut *tx)
                .await
                .map_err(DbError::from)?;
        }
        if let Some(user) = sqlx::query_as!(User,
            "SELECT u.id,u.email,u.tenant_id,u.admin FROM user_identities i JOIN users u ON u.id=i.user_id WHERE i.subject=$1 FOR UPDATE OF u",subject)
            .fetch_optional(&mut *tx).await.map_err(DbError::from)? {
            tx.commit().await.map_err(DbError::from)?;
            return Ok(user);
        }
        let user = sqlx::query_as!(
            User,
            "SELECT id,email,tenant_id,admin FROM users WHERE email=$1 FOR UPDATE",
            email
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::from)?;
        let created = user.is_none();
        let user = match user {
            Some(user) => user,
            None => {
                if !provision {
                    return Err(IdentityError::AccountRequired);
                }
                allowed_email(&email).ok_or(IdentityError::NotAllowed)?;
                let tenant = sqlx::query_scalar!("INSERT INTO tenants DEFAULT VALUES RETURNING id")
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(DbError::from)?;
                // An invalid PHC marker cannot authenticate as a local password, including on rollback.
                sqlx::query_as!(User, "INSERT INTO users(email,password_hash,tenant_id,admin) VALUES($1,'identity-only',$2,false) RETURNING id,email,tenant_id,admin", email, tenant)
                    .fetch_one(&mut *tx).await.map_err(DbError::from)?
            }
        };
        sqlx::query!(
            "INSERT INTO user_identities(subject,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING",
            subject,
            user.id
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        // Another first request may have bound this subject to its canonical account.
        let linked = sqlx::query_as!(User,
            "SELECT u.id,u.email,u.tenant_id,u.admin FROM user_identities i JOIN users u ON u.id=i.user_id WHERE i.subject=$1",subject)
            .fetch_optional(&mut *tx).await.map_err(DbError::from)?
            .ok_or(IdentityError::Conflict)?;
        if created && linked.id != user.id {
            return Err(IdentityError::Conflict);
        }
        tx.commit().await.map_err(DbError::from)?;
        Ok(linked)
    }
}

impl Sessions {
    /// Call only after resolving SDK-verified claims to the local subject binding.
    pub async fn create_identity_session(
        &self,
        subject: &str,
    ) -> Result<Option<SecretString>, IdentityError> {
        let token = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        let hash = identity_session_hash(&token);
        let inserted = sqlx::query!("INSERT INTO identity_sessions(token_hash,subject) SELECT $1,subject FROM user_identities WHERE subject=$2", hash, subject)
            .execute(&self.db.0).await.map_err(DbError::from)?.rows_affected();
        Ok((inserted == 1).then(|| token.into()))
    }

    pub async fn identity_user(&self, token: &str) -> Result<Option<User>, IdentityError> {
        if token.len() != 43
            || !token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Ok(None);
        }
        let hash = identity_session_hash(token);
        sqlx::query_as!(User, "SELECT u.id,u.email,u.tenant_id,u.admin FROM identity_sessions s JOIN user_identities i ON i.subject=s.subject JOIN users u ON u.id=i.user_id WHERE s.token_hash=$1 AND s.expires_at>now()", hash)
            .fetch_optional(&self.db.0).await.map_err(|e| IdentityError::Database(DbError::from(e)))
    }

    pub async fn logout_identity(&self, token: &str) -> Result<(), IdentityError> {
        let hash = identity_session_hash(token);
        sqlx::query!("DELETE FROM identity_sessions WHERE token_hash=$1", hash)
            .execute(&self.db.0)
            .await
            .map_err(DbError::from)?;
        Ok(())
    }
}

fn identity_session_hash(token: &str) -> Vec<u8> {
    Sha256::new()
        .chain_update(b"codemode:identity-session:v1\0")
        .chain_update(token.as_bytes())
        .finalize()
        .to_vec()
}
