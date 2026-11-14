use super::*;
use secrecy::ExposeSecret;
use subtle::ConstantTimeEq;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ClientMetadata {
    #[serde(default = "default_name")]
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    #[serde(default = "default_method")]
    pub token_endpoint_auth_method: AuthMethod,
    #[serde(default = "default_grants")]
    pub grant_types: Vec<String>,
    #[serde(default = "default_responses")]
    pub response_types: Vec<String>,
}
fn default_name() -> String {
    "MCP client".into()
}
fn default_method() -> AuthMethod {
    AuthMethod::ClientSecretBasic
}
fn default_grants() -> Vec<String> {
    vec!["authorization_code".into()]
}
fn default_responses() -> Vec<String> {
    vec!["code".into()]
}

pub struct Registration {
    pub client_id: String,
    pub metadata: ClientMetadata,
    pub client_secret: Option<SecretString>,
}

impl ClientMetadata {
    pub fn validate(&self) -> Result<(), Error> {
        if self.client_name.trim().is_empty()
            || self.client_name.len() > 128
            || self.client_name.chars().any(char::is_control)
            || self.redirect_uris.is_empty()
            || self.redirect_uris.len() > 10
            || self.redirect_uris.iter().any(|uri| !redirect_uri(uri))
            || !self.grant_types.iter().any(|g| g == "authorization_code")
            || self.grant_types.len() > 2
            || self
                .grant_types
                .iter()
                .any(|g| g != "authorization_code" && g != "refresh_token")
            || self.response_types != ["code"]
        {
            return Err(Error::Client);
        }
        Ok(())
    }
    pub(super) fn accepts_redirect(&self, requested: &str) -> bool {
        if !redirect_uri(requested) {
            return false;
        }
        self.redirect_uris.iter().any(|registered| {
            if registered == requested {
                return true;
            }
            let (Ok(registered_url), Ok(requested_url)) =
                (url::Url::parse(registered), url::Url::parse(requested))
            else {
                return false;
            };
            if !loopback(&registered_url) || !loopback(&requested_url) {
                return false;
            }
            without_loopback_port(registered) == without_loopback_port(requested)
        })
    }
}

fn without_loopback_port(value: &str) -> String {
    let Some((scheme, _)) = value.split_once("://") else {
        return value.into();
    };
    let end = value[scheme.len() + 3..]
        .find(['/', '?'])
        .map_or(value.len(), |offset| scheme.len() + 3 + offset);
    if let Some((host, port)) = value[..end].rsplit_once(':')
        && !port.is_empty()
        && port.bytes().all(|b| b.is_ascii_digit())
    {
        return format!("{host}{}", &value[end..]);
    }
    value.into()
}

impl OAuth {
    pub async fn register(&self, metadata: ClientMetadata) -> Result<Registration, Error> {
        self.allow_registration()?;
        metadata.validate()?;
        let client_id = random("cm_client_");
        let secret =
            (metadata.token_endpoint_auth_method != AuthMethod::None).then(|| random("cms_"));
        let hash = secret.as_deref().map(digest);
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("LOCK TABLE oauth_clients IN SHARE ROW EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
        let count = sqlx::query_scalar!("SELECT count(*) AS \"count!\" FROM oauth_clients")
            .fetch_one(&mut *tx)
            .await
            .map_err(DbError::from)?;
        if count >= 1024 {
            return Err(Error::Busy);
        }
        sqlx::query!(
            "INSERT INTO oauth_clients(client_id,metadata,secret_hash) VALUES($1,$2,$3)",
            client_id,
            sqlx::types::Json(&metadata) as _,
            hash
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(Registration {
            client_id,
            metadata,
            client_secret: secret.map(Into::into),
        })
    }

    pub(super) async fn client(&self, id: &str) -> Result<ClientMetadata, Error> {
        if id.len() > 2048 {
            return Err(Error::Client);
        }
        sqlx::query_scalar!("SELECT metadata AS \"metadata!: sqlx::types::Json<ClientMetadata>\" FROM oauth_clients WHERE client_id=$1",id)
            .fetch_optional(&self.db.0).await.map_err(DbError::from)?
            .map(|m|m.0).ok_or(Error::Client)
    }

    pub(super) async fn authenticate_client(
        &self,
        auth: &ClientAuthentication,
    ) -> Result<(), Error> {
        if auth.client_id.len() > 2048
            || auth
                .secret
                .as_ref()
                .is_some_and(|s| s.expose_secret().len() > 1024)
        {
            return Err(Error::Client);
        }
        let row=sqlx::query!("SELECT metadata AS \"metadata!: sqlx::types::Json<ClientMetadata>\",secret_hash FROM oauth_clients WHERE client_id=$1",auth.client_id)
            .fetch_optional(&self.db.0).await.map_err(DbError::from)?.ok_or(Error::Client)?;
        if row.metadata.token_endpoint_auth_method != auth.method {
            return Err(Error::Client);
        }
        match (auth.method, auth.secret.as_ref(), row.secret_hash) {
            (AuthMethod::None, None, None) => Ok(()),
            (
                AuthMethod::ClientSecretBasic | AuthMethod::ClientSecretPost,
                Some(secret),
                Some(hash),
            ) if bool::from(digest(secret.expose_secret()).ct_eq(&hash)) => Ok(()),
            _ => Err(Error::Client),
        }
    }
}

fn redirect_uri(value: &str) -> bool {
    if value.len() > 2048 {
        return false;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    (url.scheme() == "https" || loopback(&url))
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && !value.chars().any(|c| c.is_control() || c == '\\')
}
fn loopback(url: &url::Url) -> bool {
    url.scheme() == "http" && matches!(url.host(),Some(url::Host::Ipv4(ip)) if ip.is_loopback())
        || url.scheme() == "http"
            && matches!(url.host(),Some(url::Host::Ipv6(ip)) if ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redirect_matching_is_exact_except_for_native_loopback_ports() {
        let mut client: ClientMetadata = serde_json::from_value(
            serde_json::json!({"redirect_uris":["https://client.example/callback"]}),
        )
        .unwrap();
        assert_eq!(
            client.token_endpoint_auth_method,
            AuthMethod::ClientSecretBasic
        );
        assert!(client.accepts_redirect("https://client.example/callback"));
        assert!(!client.accepts_redirect("https://client.example/callback/"));
        assert!(!client.accepts_redirect("https://client.example:444/callback"));
        for registered in [
            "http://127.0.0.1:5000/callback",
            "http://[::1]:5000/callback",
        ] {
            client.redirect_uris = vec![registered.into()];
            assert!(client.validate().is_ok());
            assert!(client.accepts_redirect(&registered.replace("5000", "6000")));
            assert!(!client.accepts_redirect(&registered.replace("callback", "other")));
            assert!(!client.accepts_redirect(&registered.replace("callback", "%63allback")));
        }
        client.redirect_uris = vec!["http://127.0.0.1:5000/callback".into()];
        assert!(!client.accepts_redirect("http://127.1:6000/callback"));
        for invalid in [
            "javascript:alert(1)",
            "http://localhost/callback",
            "http://evil.example/callback",
            "https://user@client.example/callback",
            "https://client.example/callback#fragment",
        ] {
            client.redirect_uris = vec![invalid.into()];
            assert!(client.validate().is_err());
        }
    }
}
