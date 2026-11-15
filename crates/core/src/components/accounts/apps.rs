use super::*;
use crate::components::providers::{self, Entry, Method};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OAuthAppInput {
    pub client_id: SecretString,
    pub client_secret: SecretString,
}
impl OAuthAppInput {
    fn validate(&self) -> Result<(), Error> {
        for (value, limit) in [(&self.client_id, 512), (&self.client_secret, 4096)] {
            let value = value.expose_secret();
            if value.is_empty()
                || value.len() > limit
                || value.chars().any(|c| c.is_control() || c.is_whitespace())
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}
fn provider(name: &str) -> Result<Entry, Error> {
    providers::registry()?
        .into_iter()
        .find(|entry| {
            entry.slug == name && entry.methods.contains(&Method::OAuthApp) && entry.oauth.is_some()
        })
        .ok_or(Error::Provider)
}
impl Accounts {
    pub async fn configured_apps(&self) -> Result<Vec<String>, Error> {
        Ok(
            sqlx::query_scalar!("SELECT provider FROM oauth_apps ORDER BY provider")
                .fetch_all(&self.db.0)
                .await
                .map_err(DbError::from)?,
        )
    }
    pub async fn save_oauth_app(&self, name: &str, input: OAuthAppInput) -> Result<(), Error> {
        let provider = provider(name)?;
        input.validate()?;
        let key = self.key.as_ref().ok_or(Error::Disabled)?;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        // Serialize rare app updates so ciphertext is sealed against the retained row ID.
        sqlx::query!("LOCK TABLE oauth_apps IN SHARE ROW EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
        let id = sqlx::query_scalar!(
            "SELECT id FROM oauth_apps WHERE provider=$1 FOR UPDATE",
            name
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::from)?
        .unwrap_or_else(Uuid::new_v4);
        let plaintext = SecretString::from(
            serde_json::to_string(&(
                input.client_id.expose_secret(),
                input.client_secret.expose_secret(),
            ))
            .map_err(|_| Error::Invalid)?,
        );
        let kind = format!("oauth-app:{name}");
        let (ciphertext, nonce) = seal(
            key.expose_secret(),
            id,
            &kind,
            &self.key_id,
            plaintext.expose_secret(),
        )?;
        sqlx::query!("INSERT INTO oauth_apps(id,provider,ciphertext,nonce,key_id) VALUES($1,$2,$3,$4,$5) ON CONFLICT(tenant_id,provider) DO UPDATE SET ciphertext=EXCLUDED.ciphertext,nonce=EXCLUDED.nonce,key_id=EXCLUDED.key_id,updated_at=now()",id,name,ciphertext,nonce.as_slice(),self.key_id).execute(&mut *tx).await.map_err(DbError::from)?;
        // Changing app identity invalidates pending callbacks, while linked credentials remain intact.
        sqlx::query!("UPDATE sources SET revision=revision+1 WHERE kind='mcp_http' AND rtrim(config->>'url','/')=rtrim($1,'/')",provider.url).execute(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
    pub async fn remove_oauth_app(&self, name: &str) -> Result<(), Error> {
        let provider = provider(name)?;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("DELETE FROM oauth_apps WHERE provider=$1", name)
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
        sqlx::query!("UPDATE sources SET revision=revision+1 WHERE kind='mcp_http' AND rtrim(config->>'url','/')=rtrim($1,'/')",provider.url).execute(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
    pub(super) async fn app_for_url(
        &self,
        url: &str,
    ) -> Result<Option<(Entry, OAuthAppInput)>, Error> {
        let Some(entry) = providers::registry()?.into_iter().find(|entry| {
            entry.oauth.is_some() && entry.url.trim_end_matches('/') == url.trim_end_matches('/')
        }) else {
            return Ok(None);
        };
        let Some(row) = sqlx::query!(
            "SELECT id,ciphertext,nonce,key_id FROM oauth_apps WHERE provider=$1",
            entry.slug
        )
        .fetch_optional(&self.db.0)
        .await
        .map_err(DbError::from)?
        else {
            return Ok(None);
        };
        if row.key_id != self.key_id {
            return Err(Error::Authentication);
        }
        let key = self.key.as_ref().ok_or(Error::Disabled)?;
        let nonce = row
            .nonce
            .as_slice()
            .try_into()
            .map_err(|_| Error::Authentication)?;
        let plaintext = open(
            key.expose_secret(),
            row.id,
            &format!("oauth-app:{}", entry.slug),
            &row.key_id,
            &nonce,
            &row.ciphertext,
        )?;
        let (client_id, client_secret) =
            serde_json::from_str(plaintext.expose_secret()).map_err(|_| Error::Authentication)?;
        Ok(Some((
            entry,
            OAuthAppInput {
                client_id,
                client_secret,
            },
        )))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_configuration_rejects_empty_oversized_or_control_characters() {
        for (id, secret) in [
            (String::new(), "secret".into()),
            ("client".into(), "".into()),
            ("client\n".into(), "secret".into()),
            ("client".into(), "a".repeat(4097)),
        ] {
            assert!(matches!(
                OAuthAppInput {
                    client_id: SecretString::from(id),
                    client_secret: SecretString::from(secret)
                }
                .validate(),
                Err(Error::Invalid)
            ));
        }
    }
}
