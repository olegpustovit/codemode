use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HttpCredential {
    Bearer {
        credential: SecretString,
    },
    ApiKeyHeader {
        name: String,
        credential: SecretString,
    },
    ApiKeyQuery {
        name: String,
        credential: SecretString,
    },
    Basic {
        username: String,
        password: SecretString,
    },
}
#[derive(Clone, Copy, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Location {
    Header,
    Query,
}
#[derive(Deserialize)]
pub(crate) struct RestSecret {
    location: Location,
    name: String,
    value: SecretString,
}
impl RestSecret {
    pub fn redact(&self, message: &str) -> String {
        let value = self.value.expose_secret();
        let mut message = message.replace(value, "[REDACTED]");
        if let Some(token) = value.strip_prefix("Bearer ") {
            message = message.replace(token, "[REDACTED]");
        }
        if matches!(self.location, Location::Query) {
            let encoded = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("", value)
                .finish();
            if let Some(encoded) = encoded.strip_prefix('=') {
                message = message.replace(encoded, "[REDACTED]");
            }
        }
        if let Some(encoded) = value.strip_prefix("Basic ") {
            message = message.replace(encoded, "[REDACTED]");
            if let Ok(decoded) = STANDARD.decode(encoded)
                && let Ok(decoded) = String::from_utf8(decoded)
            {
                let decoded = SecretString::from(decoded);
                if let Some((username, password)) = decoded.expose_secret().split_once(':') {
                    for part in [username, password].into_iter().filter(|s| !s.is_empty()) {
                        message = message.replace(part, "[REDACTED]");
                    }
                }
            }
        }
        message
    }
    pub fn apply(&self, request: &mut reqwest::Request) -> Result<(), Error> {
        match self.location {
            Location::Header => {
                let name = reqwest::header::HeaderName::from_bytes(self.name.as_bytes())
                    .map_err(|_| Error::Invalid)?;
                let mut value = reqwest::header::HeaderValue::from_str(self.value.expose_secret())
                    .map_err(|_| Error::Invalid)?;
                value.set_sensitive(true);
                request.headers_mut().insert(name, value);
            }
            Location::Query => {
                let pairs = request
                    .url()
                    .query_pairs()
                    .filter(|(name, _)| name != self.name.as_str())
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect::<Vec<_>>();
                request.url_mut().set_query(None);
                request
                    .url_mut()
                    .query_pairs_mut()
                    .extend_pairs(pairs)
                    .append_pair(&self.name, self.value.expose_secret());
            }
        }
        Ok(())
    }
}
impl Accounts {
    pub async fn link_http(&self, source: Uuid, credential: HttpCredential) -> Result<(), Error> {
        let (location, name, value) = match credential {
            HttpCredential::Bearer { credential } => {
                if !valid_bearer(credential.expose_secret()) {
                    return Err(Error::Invalid);
                }
                (
                    Location::Header,
                    "authorization".into(),
                    SecretString::from(format!("Bearer {}", credential.expose_secret())),
                )
            }
            HttpCredential::Basic { username, password } => {
                if username.len() > 1024
                    || username.contains(':')
                    || username.chars().any(char::is_control)
                    || password.expose_secret().chars().any(char::is_control)
                {
                    return Err(Error::Invalid);
                }
                let raw = SecretString::from(format!("{username}:{}", password.expose_secret()));
                (
                    Location::Header,
                    "authorization".into(),
                    SecretString::from(format!("Basic {}", STANDARD.encode(raw.expose_secret()))),
                )
            }
            HttpCredential::ApiKeyHeader { name, credential } => {
                if (super::super::openapi::controlled_header(&name)
                    && !matches!(
                        name.to_ascii_lowercase().as_str(),
                        "authorization" | "cookie"
                    ))
                    || reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err()
                {
                    return Err(Error::Invalid);
                }
                (Location::Header, name, credential)
            }
            HttpCredential::ApiKeyQuery { name, credential } => (Location::Query, name, credential),
        };
        if name.is_empty()
            || name.len() > 256
            || name.chars().any(char::is_control)
            || value.expose_secret().is_empty()
            || value.expose_secret().len() > 65536
            || value.expose_secret().contains('\0')
        {
            return Err(Error::Invalid);
        }
        if matches!(location, Location::Header)
            && reqwest::header::HeaderValue::from_str(value.expose_secret()).is_err()
        {
            return Err(Error::Invalid);
        }
        #[derive(serde::Serialize)]
        struct Encoded<'a> {
            location: Location,
            name: &'a str,
            value: &'a str,
        }
        let encoded = serde_json::to_string(&Encoded {
            location,
            name: &name,
            value: value.expose_secret(),
        })
        .map_err(|_| Error::Invalid)?;
        self.store(source, "rest", encoded.into()).await
    }
    pub(crate) async fn rest(
        &self,
        source: Uuid,
        server: &str,
    ) -> Result<Option<RestSecret>, Error> {
        let row=sqlx::query!("SELECT CASE WHEN s.kind='openapi' THEN s.config->>'serverUrl' ELSE s.config->>'url' END AS server,a.kind,a.ciphertext,a.nonce,a.key_id FROM sources s LEFT JOIN accounts a ON a.source_id=s.id WHERE s.id=$1 AND s.kind IN ('openapi','graphql')",source)
            .fetch_optional(&self.db.0).await.map_err(DbError::from)?.ok_or(Error::Changed)?;
        if row.server.as_deref() != Some(server) {
            return Err(Error::Changed);
        }
        let Some(kind) = row.kind else {
            return Ok(None);
        };
        let key_id = row.key_id.ok_or(Error::Authentication)?;
        if key_id != self.key_id {
            return Err(Error::Authentication);
        }
        let nonce: [u8; 24] = row
            .nonce
            .ok_or(Error::Authentication)?
            .try_into()
            .map_err(|_| Error::Authentication)?;
        let key = self.key.as_ref().ok_or(Error::Disabled)?;
        let value = open(
            key.expose_secret(),
            source,
            &kind,
            &key_id,
            &nonce,
            &row.ciphertext.ok_or(Error::Authentication)?,
        )?;
        if kind == "bearer" {
            return Ok(Some(RestSecret {
                location: Location::Header,
                name: "authorization".into(),
                value: format!("Bearer {}", value.expose_secret()).into(),
            }));
        }
        if kind != "rest" {
            return Err(Error::Authentication);
        }
        serde_json::from_str(value.expose_secret())
            .map(Some)
            .map_err(|_| Error::Authentication)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts_plain_and_url_encoded_query_credentials() {
        let secret = RestSecret {
            location: Location::Query,
            name: "key".into(),
            value: "embedded &secret=é".into(),
        };
        assert_eq!(
            secret.redact("failed embedded &secret=é at ?key=embedded+%26secret%3D%C3%A9"),
            "failed [REDACTED] at ?key=[REDACTED]"
        );
    }
}
