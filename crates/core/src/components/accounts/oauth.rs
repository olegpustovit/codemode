use super::oauth_error::failure;
use super::*;
use OAuthStage::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::future::BoxFuture;
use oauth2::{TokenResponse, basic::BasicTokenType};
use rmcp::transport::auth::{
    AuthorizationManager, AuthorizationMetadata, CredentialStore, InMemoryCredentialStore,
    InMemoryStateStore, OAuthClientConfig, StateStore, StoredAuthorizationState, StoredCredentials,
};
use sha2::{Digest, Sha256};
use std::time::Duration;

#[derive(serde::Serialize, serde::Deserialize)]
struct Provider {
    url: String,
    resource: String,
    metadata: AuthorizationMetadata,
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: String,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Pending {
    provider: Provider,
    authorization: StoredAuthorizationState,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Linked {
    provider: Provider,
    credentials: StoredCredentials,
}
struct HttpCredential {
    kind: String,
    value: SecretString,
    nonce: Vec<u8>,
    refresh_due: bool,
}

pub enum OAuthCompletion {
    Linked(Uuid),
    Denied(Uuid, String),
}

impl Provider {
    async fn authorize(self) -> Result<(String, String, Pending), Error> {
        let states = InMemoryStateStore::new();
        let mut manager = self.manager(InMemoryCredentialStore::new()).await?;
        manager.set_state_store(states.clone());
        let scopes = self.metadata.scopes_supported.clone().unwrap_or_default();
        let refs: Vec<_> = scopes.iter().map(String::as_str).collect();
        let url = manager
            .get_authorization_url(&refs)
            .await
            .map_err(|error| {
                failure(
                    Consent,
                    error,
                    &[self.client_secret.as_deref().unwrap_or("")],
                )
            })?;
        let parsed = url::Url::parse(&url)
            .map_err(|_| failure(Consent, "invalid authorization URL", &[]))?;
        let csrf = parsed
            .query_pairs()
            .find(|(name, _)| name == "state")
            .map(|(_, value)| value.into_owned())
            .ok_or_else(|| failure(Consent, "authorization state missing", &[]))?;
        let authorization = states
            .load(&csrf)
            .await
            .map_err(|error| failure(Consent, error, &[]))?
            .ok_or_else(|| failure(Consent, "authorization state missing", &[]))?;
        Ok((
            url,
            csrf,
            Pending {
                provider: self,
                authorization,
            },
        ))
    }
    async fn manager(
        &self,
        credentials: InMemoryCredentialStore,
    ) -> Result<AuthorizationManager, Error> {
        let mut manager = AuthorizationManager::new(&self.resource)
            .await
            .map_err(|_| {
                failure(
                    Registration,
                    "stored OAuth client configuration is invalid",
                    &[],
                )
            })?;
        manager.set_metadata(self.metadata.clone());
        let mut client = OAuthClientConfig::new(&self.client_id, &self.redirect_uri)
            .with_application_type("web");
        client.client_secret = self.client_secret.clone();
        manager.configure_client(client).map_err(|_| {
            failure(
                Registration,
                "stored OAuth client configuration is invalid",
                &[],
            )
        })?;
        manager.set_credential_store(credentials);
        Ok(manager)
    }
}

impl Accounts {
    pub fn start_oauth<'a>(
        &'a self,
        source: Uuid,
        hostname: &'a str,
        session: &'a str,
    ) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            let _permit = self.authorizations.try_acquire().map_err(|_| Error::Busy)?;
            let key = self.key.as_ref().ok_or(Error::Disabled)?;
            let record=sqlx::query!("SELECT revision,config->>'url' AS url FROM sources WHERE id=$1 AND kind='mcp_http'",source)
                .fetch_optional(&self.db.0).await.map_err(DbError::from)?.ok_or(Error::NotFound)?;
            let url = record.url.ok_or(Error::Invalid)?;
            let redirect_uri = format!("https://{hostname}/api/accounts/oauth/callback");
            let metadata_url = format!("https://{hostname}/oauth/client-metadata.json");
            let static_app = self.app_for_url(&url).await?;
            let state_store = InMemoryStateStore::new();
            let mut stage = Probe;
            let discovery = async {
                if let Some((entry, app)) = static_app {
                    stage = Consent;
                    let endpoints = entry.oauth.ok_or(Error::Provider)?;
                    let mut metadata = AuthorizationMetadata::default();
                    metadata.authorization_endpoint = endpoints.authorization_endpoint;
                    metadata.token_endpoint = endpoints.token_endpoint;
                    metadata.scopes_supported = Some(endpoints.scopes);
                    metadata.response_types_supported = Some(vec!["code".into()]);
                    metadata.code_challenge_methods_supported = Some(vec!["S256".into()]);
                    metadata.additional_fields.insert(
                        "token_endpoint_auth_methods_supported".into(),
                        serde_json::json!(["client_secret_post"]),
                    );
                    let provider = Provider {
                        url: url.clone(),
                        resource: url.clone(),
                        metadata,
                        client_id: app.client_id.expose_secret().to_owned(),
                        client_secret: Some(app.client_secret.expose_secret().to_owned()),
                        redirect_uri,
                    };
                    return provider.authorize().await;
                }
                let mut manager = AuthorizationManager::new(&url)
                    .await
                    .map_err(|error| failure(stage, error, &[&url]))?;
                let probe = reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_secs(5))
                    .build()
                    .map_err(|error| failure(stage, error, &[&url]))?;
                let response = probe
                    .get(&url)
                    .header("accept", "application/json, text/event-stream")
                    .send()
                    .await
                    .map_err(|error| failure(stage, error, &[&url]))?;
                let challenge = (response.status() == reqwest::StatusCode::UNAUTHORIZED)
                    .then(|| {
                        response
                            .headers()
                            .get(reqwest::header::WWW_AUTHENTICATE)
                            .and_then(|h| h.to_str().ok())
                    })
                    .flatten();
                stage = ServerMetadata;
                let resolution = manager
                    .resolve_metadata_from_challenge(challenge)
                    .await
                    .map_err(|error| {
                        let stage = if matches!(error, rmcp::transport::auth::AuthError::ProtectedResourceMetadataInvalid { .. }) { ResourceMetadata } else { ServerMetadata };
                        failure(stage, error, &[&url])
                    })?;
                let metadata = resolution.metadata;
                let cimd = metadata
                    .additional_fields
                    .get("client_id_metadata_document_supported")
                    .and_then(|v| v.as_bool())
                    == Some(true);
                manager.set_metadata(metadata.clone());
                manager.set_state_store(state_store.clone());
                let scopes = manager.select_initial_scopes(&[]).await;
                let scope_refs: Vec<_> = scopes.iter().map(String::as_str).collect();
                let identity = OAuthClientConfig::new(&metadata_url, &redirect_uri)
                    .with_application_type("web")
                    .with_scopes(scopes.clone());
                // The SDK takes application_type from configuration before making its DCR request.
                stage = Registration;
                manager
                    .configure_client(identity.clone())
                    .map_err(|error| failure(stage, error, &[&url]))?;
                let client = if cimd {
                    identity
                } else {
                    manager
                        .register_client("Codemode Gateway", &redirect_uri, &scope_refs)
                        .await
                        .map_err(|error| failure(stage, error, &[&url]))?
                };
                stage = Consent;
                let authorization_url = manager
                    .get_authorization_url(&scope_refs)
                    .await
                    .map_err(|error| failure(stage, error, &[&url]))?;
                let parsed = url::Url::parse(&authorization_url)
                    .map_err(|error| failure(stage, error, &[&url]))?;
                let csrf = parsed
                    .query_pairs()
                    .find(|(name, _)| name == "state")
                    .map(|(_, value)| value.into_owned())
                    .ok_or_else(|| {
                        failure(stage, "provider authorization state is incomplete", &[])
                    })?;
                let resource = parsed
                    .query_pairs()
                    .find(|(name, _)| name == "resource")
                    .map(|(_, value)| value.into_owned())
                    .ok_or_else(|| {
                        failure(stage, "provider authorization state is incomplete", &[])
                    })?;
                let authorization = state_store
                    .load(&csrf)
                    .await
                    .map_err(|error| failure(stage, error, &[&url]))?
                    .ok_or_else(|| {
                        failure(stage, "provider authorization state is incomplete", &[])
                    })?;
                let provider = Provider {
                    url: url.clone(),
                    resource,
                    metadata,
                    client_id: client.client_id,
                    client_secret: client.client_secret,
                    redirect_uri,
                };
                Ok::<_, Error>((
                    authorization_url,
                    csrf,
                    Pending {
                        provider,
                        authorization,
                    },
                ))
            };
            let outcome = tokio::time::timeout(Duration::from_secs(25), discovery).await;
            let (authorization_url, csrf, pending) =
                outcome.map_err(|_| failure(stage, "provider request timed out", &[]))??;
            let encoded = encode(&pending)?;
            let (ciphertext, nonce) = seal(
                key.expose_secret(),
                source,
                "oauth-state",
                &self.key_id,
                encoded.expose_secret(),
            )?;
            let hash = Sha256::digest(csrf.as_bytes()).to_vec();
            let session = Sha256::digest(session.as_bytes()).to_vec();
            let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
            sqlx::query!("SELECT id FROM sources WHERE id=$1 AND revision=$2 AND config->>'url'=$3 FOR UPDATE",source,record.revision,url)
                .fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::Changed)?;
            sqlx::query!(
                "DELETE FROM upstream_oauth_states WHERE source_id=$1",
                source
            )
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
            sqlx::query!("INSERT INTO upstream_oauth_states(state_hash,session_hash,source_id,source_revision,ciphertext,nonce,key_id) VALUES($1,$2,$3,$4,$5,$6,$7)",hash,session,source,record.revision,ciphertext,nonce.as_slice(),self.key_id)
                .execute(&mut *tx).await.map_err(DbError::from)?;
            tx.commit().await.map_err(DbError::from)?;
            Ok(authorization_url)
        })
    }

    pub fn complete_oauth<'a>(
        &'a self,
        state: &'a str,
        session: &'a str,
        code: Option<&'a str>,
        issuer: Option<&'a str>,
        denied: Option<&'a str>,
    ) -> BoxFuture<'a, Result<OAuthCompletion, Error>> {
        Box::pin(async move {
            if state.len() > 256
                || code.is_some_and(|c| c.is_empty() || c.len() > 8192)
                || issuer.is_some_and(|i| i.len() > 2048)
            {
                return Err(Error::State);
            }
            let _permit = self.authorizations.try_acquire().map_err(|_| Error::Busy)?;
            let hash = Sha256::digest(state.as_bytes()).to_vec();
            let session = Sha256::digest(session.as_bytes()).to_vec();
            let row=sqlx::query!("SELECT source_id,source_revision,ciphertext,nonce,key_id FROM upstream_oauth_states WHERE state_hash=$1 AND session_hash=$2 AND expires_at>now()",hash,session)
                .fetch_optional(&self.db.0).await.map_err(DbError::from)?.ok_or(Error::State)?;
            let pending: Pending = self.decrypt(
                row.source_id,
                "oauth-state",
                &row.key_id,
                &row.nonce,
                &row.ciphertext,
            )?;
            if pending.authorization.csrf_token != state
                || (pending.authorization.require_issuer && issuer.is_none())
                || matches!((&pending.authorization.expected_issuer,issuer),(Some(expected),Some(actual)) if expected!=actual)
            {
                return Err(Error::State);
            }
            if code.is_none() && denied.is_none() {
                return Err(Error::State);
            }
            let taken=sqlx::query!("DELETE FROM upstream_oauth_states WHERE state_hash=$1 AND session_hash=$2 AND expires_at>now()",hash,session)
                .execute(&self.db.0).await.map_err(DbError::from)?.rows_affected();
            if taken != 1 {
                return Err(Error::State);
            }
            let verifier = pending.authorization.pkce_verifier.clone();
            let basic = STANDARD.encode(format!(
                "{}:{}",
                pending.provider.client_id,
                pending.provider.client_secret.as_deref().unwrap_or("")
            ));
            let secrets = [
                code.unwrap_or(""),
                state,
                verifier.as_str(),
                pending.provider.client_secret.as_deref().unwrap_or(""),
                basic.as_str(),
            ];
            if let Some(reason) = denied {
                let error = failure(Consent, reason, &secrets);
                tracing::warn!(error=%error,"upstream authorization declined");
                return Ok(OAuthCompletion::Denied(row.source_id, error.to_string()));
            }
            let current=sqlx::query_scalar!("SELECT EXISTS(SELECT 1 FROM sources WHERE id=$1 AND revision=$2 AND config->>'url'=$3) AS \"current!\"",row.source_id,row.source_revision,pending.provider.url)
                .fetch_one(&self.db.0).await.map_err(DbError::from)?;
            if !current {
                return Err(Error::Changed);
            }
            let store = InMemoryCredentialStore::new();
            let mut manager = pending.provider.manager(store.clone()).await?;
            let states = InMemoryStateStore::new();
            states
                .save(state, pending.authorization)
                .await
                .map_err(|error| failure(Store, error, &secrets))?;
            manager.set_state_store(states);
            tokio::time::timeout(
                Duration::from_secs(25),
                manager.exchange_code_for_token_with_issuer(
                    code.ok_or(Error::State)?,
                    state,
                    issuer,
                ),
            )
            .await
            .map_err(|_| failure(Exchange, "provider request timed out", &secrets))?
            .map_err(|error| failure(Exchange, error, &secrets))?;
            let credentials = store
                .load()
                .await
                .map_err(|error| failure(Store, error, &secrets))?
                .ok_or_else(|| failure(Store, "provider did not return credentials", &[]))?;
            validate_credentials(&credentials)?;
            let linked = Linked {
                provider: pending.provider,
                credentials,
            };
            self.save_oauth(row.source_id, &linked, None, Some(row.source_revision))
                .await?;
            Ok(OAuthCompletion::Linked(row.source_id))
        })
    }

    pub async fn sweep_oauth(&self) -> Result<(), Error> {
        sqlx::query!("DELETE FROM upstream_oauth_states WHERE expires_at<=now()")
            .execute(&self.db.0)
            .await
            .map_err(DbError::from)?;
        Ok(())
    }

    pub(crate) async fn bearer(
        &self,
        source: Uuid,
        uri: &str,
    ) -> Result<Option<SecretString>, Error> {
        let Some(credential) = self.http_credential(source, uri).await? else {
            return Ok(None);
        };
        if credential.kind == "bearer" {
            return Ok(Some(credential.value));
        }
        if credential.kind != "oauth" {
            return Err(Error::Authentication);
        }
        let linked: Linked = serde_json::from_str(credential.value.expose_secret())
            .map_err(|_| Error::Authentication)?;
        if linked.provider.url != uri {
            return Err(Error::Changed);
        }
        validate_credentials(&linked.credentials)?;
        if !credential.refresh_due {
            return Ok(Some(access(&linked.credentials)?));
        }
        self.refresh_oauth(source, uri).await.map(Some)
    }

    fn refresh_oauth<'a>(
        &'a self,
        source: Uuid,
        uri: &'a str,
    ) -> BoxFuture<'a, Result<SecretString, Error>> {
        Box::pin(async move {
            // Fixed stripes bound coordination memory even as sources are deleted and recreated.
            let _guard = self.refresh_locks[(source.as_u128() % 64) as usize]
                .lock()
                .await;
            let Some(credential) = self.http_credential(source, uri).await? else {
                return Err(Error::Changed);
            };
            if credential.kind != "oauth" {
                return Err(Error::Changed);
            }
            let mut linked: Linked = serde_json::from_str(credential.value.expose_secret())
                .map_err(|_| Error::Authentication)?;
            if linked.provider.url != uri {
                return Err(Error::Changed);
            }
            validate_credentials(&linked.credentials)?;
            if !credential.refresh_due {
                return access(&linked.credentials);
            }
            let basic = STANDARD.encode(format!(
                "{}:{}",
                linked.provider.client_id,
                linked.provider.client_secret.as_deref().unwrap_or("")
            ));
            let token = linked
                .credentials
                .token_response
                .as_ref()
                .ok_or(Error::Authentication)?;
            let secrets = [
                linked.provider.client_secret.as_deref().unwrap_or(""),
                basic.as_str(),
                token.access_token().secret(),
                token
                    .refresh_token()
                    .map(|t| t.secret().as_str())
                    .unwrap_or(""),
            ];
            let store = InMemoryCredentialStore::new();
            store
                .save(linked.credentials.clone())
                .await
                .map_err(|error| failure(Refresh, error, &secrets))?;
            let manager = linked.provider.manager(store.clone()).await?;
            let result = tokio::time::timeout(Duration::from_secs(20), manager.refresh_token())
                .await
                .map_err(|error| failure(Refresh, error, &secrets))?;
            match result {
                Ok(_) => {}
                Err(
                    error @ (rmcp::transport::auth::AuthError::AuthorizationRequired
                    | rmcp::transport::auth::AuthError::TokenRefreshRejected(_)),
                ) => {
                    self.reject_oauth(source, uri, &credential.nonce).await?;
                    return Err(failure(Refresh, error, &secrets));
                }
                Err(error) => return Err(failure(Refresh, error, &secrets)),
            }
            linked.credentials = store
                .load()
                .await
                .map_err(|error| failure(Refresh, error, &secrets))?
                .ok_or_else(|| failure(Store, "refreshed credentials were not stored", &[]))?;
            validate_credentials(&linked.credentials)?;
            self.save_oauth(source, &linked, Some(&credential.nonce), None)
                .await?;
            access(&linked.credentials)
        })
    }

    async fn http_credential(
        &self,
        source: Uuid,
        uri: &str,
    ) -> Result<Option<HttpCredential>, Error> {
        let row=sqlx::query!("SELECT s.config->>'url' AS url,a.kind,a.ciphertext,a.nonce,a.key_id,COALESCE(a.kind='oauth' AND a.expires_at<now()+interval '30 seconds',false) AS \"refresh_due!\" FROM sources s LEFT JOIN accounts a ON a.source_id=s.id WHERE s.id=$1 AND s.kind='mcp_http'",source)
            .fetch_optional(&self.db.0).await.map_err(DbError::from)?.ok_or(Error::Changed)?;
        if row.url.as_deref() != Some(uri) {
            return Err(Error::Changed);
        }
        let Some(kind) = row.kind else {
            return Ok(None);
        };
        let key_id = row.key_id.ok_or(Error::Authentication)?;
        let nonce = row.nonce.ok_or(Error::Authentication)?;
        let ciphertext = row.ciphertext.ok_or(Error::Authentication)?;
        if key_id != self.key_id {
            return Err(Error::Authentication);
        }
        let key = self.key.as_ref().ok_or(Error::Disabled)?;
        let raw_nonce: [u8; 24] = nonce
            .as_slice()
            .try_into()
            .map_err(|_| Error::Authentication)?;
        let value = open(
            key.expose_secret(),
            source,
            &kind,
            &key_id,
            &raw_nonce,
            &ciphertext,
        )?;
        Ok(Some(HttpCredential {
            kind,
            value,
            nonce,
            refresh_due: row.refresh_due,
        }))
    }

    fn decrypt<T: serde::de::DeserializeOwned>(
        &self,
        source: Uuid,
        kind: &str,
        key_id: &str,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<T, Error> {
        if key_id != self.key_id {
            return Err(Error::Authentication);
        }
        let key = self.key.as_ref().ok_or(Error::Disabled)?;
        let nonce = nonce.try_into().map_err(|_| Error::Authentication)?;
        let plaintext = open(
            key.expose_secret(),
            source,
            kind,
            key_id,
            &nonce,
            ciphertext,
        )?;
        serde_json::from_str(plaintext.expose_secret()).map_err(|_| Error::Authentication)
    }

    async fn save_oauth(
        &self,
        source: Uuid,
        linked: &Linked,
        previous_nonce: Option<&[u8]>,
        revision: Option<i64>,
    ) -> Result<(), Error> {
        let key = self.key.as_ref().ok_or(Error::Disabled)?;
        let (ciphertext, nonce) = seal(
            key.expose_secret(),
            source,
            "oauth",
            &self.key_id,
            encode(linked)?.expose_secret(),
        )?;
        let expires = match (
            &linked.credentials.token_response,
            linked.credentials.token_received_at,
        ) {
            (Some(token), Some(at)) => token
                .expires_in()
                .and_then(|duration| at.checked_add(duration.as_secs()))
                .map(i64::try_from)
                .transpose()
                .map_err(|_| Error::Invalid)?,
            _ => None,
        };
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT id FROM sources WHERE id=$1 AND kind='mcp_http' AND config->>'url'=$2 AND ($3::bigint IS NULL OR revision=$3) FOR UPDATE",source,linked.provider.url,revision)
            .fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::Changed)?;
        if let Some(previous) = previous_nonce {
            let changed=sqlx::query!("UPDATE accounts SET ciphertext=$3,nonce=$4,key_id=$5,expires_at=to_timestamp($6::bigint::double precision),updated_at=now() WHERE source_id=$1 AND kind='oauth' AND nonce=$2",source,previous,ciphertext,nonce.as_slice(),self.key_id,expires)
                .execute(&mut *tx).await.map_err(DbError::from)?.rows_affected();
            if changed != 1 {
                return Err(Error::Changed);
            }
        } else {
            sqlx::query!("INSERT INTO accounts(source_id,kind,ciphertext,nonce,key_id,expires_at) VALUES($1,'oauth',$2,$3,$4,to_timestamp($5::bigint::double precision)) ON CONFLICT(source_id) DO UPDATE SET kind='oauth',ciphertext=EXCLUDED.ciphertext,nonce=EXCLUDED.nonce,key_id=EXCLUDED.key_id,expires_at=EXCLUDED.expires_at,updated_at=now()",source,ciphertext,nonce.as_slice(),self.key_id,expires)
                .execute(&mut *tx).await.map_err(DbError::from)?;
            sqlx::query!("UPDATE sources SET revision=revision+1,status='pending',last_error=NULL WHERE id=$1",source).execute(&mut *tx).await.map_err(DbError::from)?;
        }
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }

    async fn reject_oauth(&self, source: Uuid, uri: &str, nonce: &[u8]) -> Result<(), Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!(
            "SELECT id FROM sources WHERE id=$1 AND config->>'url'=$2 FOR UPDATE",
            source,
            uri
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::from)?
        .ok_or(Error::Changed)?;
        let removed = sqlx::query!(
            "DELETE FROM accounts WHERE source_id=$1 AND kind='oauth' AND nonce=$2",
            source,
            nonce
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?
        .rows_affected();
        if removed == 1 {
            sqlx::query!("UPDATE sources SET revision=revision+1,status='pending',last_error=NULL WHERE id=$1",source).execute(&mut *tx).await.map_err(DbError::from)?;
        }
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
}

fn encode(value: &impl serde::Serialize) -> Result<SecretString, Error> {
    let encoded: SecretString = serde_json::to_string(value)
        .map_err(|_| Error::Invalid)?
        .into();
    if encoded.expose_secret().len() > 131072 {
        return Err(Error::Invalid);
    }
    Ok(encoded)
}
fn access(credentials: &StoredCredentials) -> Result<SecretString, Error> {
    Ok(credentials
        .token_response
        .as_ref()
        .ok_or(Error::Authentication)?
        .access_token()
        .secret()
        .clone()
        .into())
}
fn validate_credentials(credentials: &StoredCredentials) -> Result<(), Error> {
    let response = credentials
        .token_response
        .as_ref()
        .ok_or(Error::Authentication)?;
    if response.token_type() != &BasicTokenType::Bearer
        || !valid_bearer(response.access_token().secret())
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oauth_dispatch_requires_a_bearer_token_safe_for_the_authorization_header() {
        let credentials = |token: &str, kind: &str| {
            let response = serde_json::from_value(
                serde_json::json!({"access_token":token,"token_type":kind,"expires_in":3600}),
            )
            .unwrap();
            StoredCredentials::new("fixture".into(), Some(response), vec![], Some(1))
        };
        assert!(validate_credentials(&credentials("opaque-access-token", "Bearer")).is_ok());
        for token in ["".into(), "bad\r\nheader".into(), "a".repeat(65537)] {
            assert!(validate_credentials(&credentials(&token, "Bearer")).is_err());
        }
        assert!(validate_credentials(&credentials("opaque-access-token", "MAC")).is_err());
    }
}

#[cfg(test)]
#[path = "static_tests.rs"]
mod static_tests;
