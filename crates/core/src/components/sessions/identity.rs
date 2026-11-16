use super::{Sessions, User, normalize_email};
use crate::infrastructure::postgres::Error as DbError;

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("invalid verified identity")]
    InvalidClaims,
    #[error("identity requires an existing local account")]
    AccountRequired,
    #[error("local account is bound to another identity")]
    Conflict,
    #[error(transparent)]
    Database(#[from] DbError),
}

impl Sessions {
    /// Accept only SDK-verified claims. Roles and tenant membership remain local.
    pub async fn link_identity(&self, subject: &str, email: &str) -> Result<User, IdentityError> {
        if subject.is_empty() || subject.len() > 255 || subject.contains('\0') {
            return Err(IdentityError::InvalidClaims);
        }
        let email = normalize_email(email).ok_or(IdentityError::InvalidClaims)?;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
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
        .map_err(DbError::from)?
        .ok_or(IdentityError::AccountRequired)?;
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
        tx.commit().await.map_err(DbError::from)?;
        Ok(linked)
    }
}
