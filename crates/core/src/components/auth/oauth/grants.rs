use super::*;
use subtle::ConstantTimeEq;

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct AuthorizationRequest {
    #[serde(default, rename = "workspaceId")]
    pub workspace_id: Option<Uuid>,
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub resource: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: Option<String>,
    pub scope: Option<String>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Consent {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uri: String,
    pub resource: String,
    pub scope: &'static str,
    pub workspace_id: Option<Uuid>,
    pub requested_workspace_id: Option<Uuid>,
    pub workspace_notice: Option<&'static str>,
}
#[derive(serde::Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    pub resource: String,
    pub code: Option<SecretString>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<SecretString>,
    pub refresh_token: Option<SecretString>,
    pub scope: Option<String>,
}
#[derive(serde::Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    pub scope: String,
}
struct Grant {
    id: Uuid,
    hostname: String,
    client_id: String,
    scope: String,
    refresh_allowed: bool,
}

impl OAuth {
    pub async fn consent(&self, request: &AuthorizationRequest) -> Result<Consent, Error> {
        validate_authorization(request)?;
        let client = self.client(&request.client_id).await?;
        if !client.accepts_redirect(&request.redirect_uri) {
            return Err(Error::Request);
        }
        let hostname = resource_host(&request.resource)?;
        if hostname != self.hostname {
            return Err(Error::Target);
        }
        let scope = resource_scope(&request.resource)?;
        let workspace = if scope == "admin" {
            sqlx::query_scalar!("SELECT id FROM endpoints WHERE all_sources")
                .fetch_optional(&self.db.0)
                .await
                .map_err(DbError::from)?
        } else if let Some(id) = request.workspace_id {
            Some(
                sqlx::query_scalar!("SELECT id FROM endpoints WHERE id=$1", id)
                    .fetch_optional(&self.db.0)
                    .await
                    .map_err(DbError::from)?
                    .ok_or(Error::Target)?,
            )
        } else {
            None
        };
        let requested = resource_workspace(&request.resource);
        let requested_workspace_id = if let Some(id) = requested {
            sqlx::query_scalar!("SELECT id FROM endpoints WHERE id=$1", id)
                .fetch_optional(&self.db.0)
                .await
                .map_err(DbError::from)?
        } else {
            None
        };
        Ok(Consent {
            client_id: request.client_id.clone(),
            client_name: client.client_name,
            redirect_uri: request.redirect_uri.clone(),
            resource: request.resource.clone(),
            scope,
            workspace_id: workspace,
            requested_workspace_id,
            workspace_notice: (requested.is_some() && requested_workspace_id.is_none()).then_some(
                "The workspace in the MCP URL is unavailable. Choose a workspace below.",
            ),
        })
    }

    // Only the session-protected browser consent handler may call this method.
    pub async fn issue_code(&self, request: &AuthorizationRequest) -> Result<String, Error> {
        let consent = self.consent(request).await?;
        let client = self.client(&request.client_id).await?;
        let workspace = consent.workspace_id.ok_or(Error::Request)?;
        self.access.workspace(workspace).await?;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!(
            "SELECT id FROM endpoints WHERE id=$1 FOR KEY SHARE",
            workspace
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::from)?
        .ok_or(Error::Target)?;
        let refresh = client.grant_types.iter().any(|g| g == "refresh_token");
        let resource = canonical_resource(&request.resource)?;
        let grant=sqlx::query_scalar!("INSERT INTO oauth_grants(client_id,endpoint_id,scope,refresh_allowed,resource_url) VALUES($1,$2,$3,$4,$5) RETURNING id",request.client_id,workspace,consent.scope,refresh,resource)
            .fetch_one(&mut *tx).await.map_err(DbError::from)?;
        let code = random("cmc_");
        sqlx::query!("INSERT INTO oauth_codes(code_hash,grant_id,redirect_uri,challenge) VALUES($1,$2,$3,$4)",digest(&code),grant,request.redirect_uri,request.code_challenge)
            .execute(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(code)
    }

    pub async fn token(
        &self,
        auth: &ClientAuthentication,
        request: &TokenRequest,
    ) -> Result<TokenResponse, Error> {
        use secrecy::ExposeSecret;
        self.authenticate_client(auth).await?;
        if resource_host(&request.resource)? != self.hostname {
            return Err(Error::Target);
        }
        if request
            .scope
            .as_deref()
            .is_some_and(|s| s != resource_scope(&request.resource).unwrap_or(""))
        {
            return Err(Error::Scope);
        }
        match request.grant_type.as_str() {
            "authorization_code" => {
                let code = request.code.as_ref().ok_or(Error::Request)?.expose_secret();
                let verifier = request
                    .code_verifier
                    .as_ref()
                    .ok_or(Error::Request)?
                    .expose_secret();
                let redirect = request.redirect_uri.as_deref().ok_or(Error::Request)?;
                if code.len() != 47 || !valid_verifier(verifier) {
                    return Err(Error::Grant);
                }
                let hash = digest(code);
                let grant = sqlx::query_scalar!(
                    "SELECT grant_id FROM oauth_codes WHERE code_hash=$1",
                    hash
                )
                .fetch_optional(&self.db.0)
                .await
                .map_err(DbError::from)?
                .ok_or(Error::Grant)?;
                self.access.grant(grant).await?;
                let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
                let grant = lock_grant(&mut tx, grant, &auth.client_id, &request.resource).await?;
                let code=sqlx::query!("SELECT redirect_uri,challenge,consumed_at IS NOT NULL AS \"consumed!\",expires_at>now() AS \"valid!\" FROM oauth_codes WHERE code_hash=$1",hash)
                    .fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::Grant)?;
                let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
                if code.redirect_uri != redirect
                    || !bool::from(challenge.as_bytes().ct_eq(code.challenge.as_bytes()))
                {
                    return Err(Error::Grant);
                }
                if code.consumed {
                    revoke_grant(&mut tx, grant.id).await?;
                    tx.commit().await.map_err(DbError::from)?;
                    return Err(Error::Grant);
                }
                if !code.valid {
                    return Err(Error::Grant);
                }
                sqlx::query!(
                    "UPDATE oauth_codes SET consumed_at=now() WHERE code_hash=$1",
                    hash
                )
                .execute(&mut *tx)
                .await
                .map_err(DbError::from)?;
                let tokens = mint(&mut tx, &grant).await?;
                tx.commit().await.map_err(DbError::from)?;
                Ok(tokens)
            }
            "refresh_token" => {
                let refresh = request
                    .refresh_token
                    .as_ref()
                    .ok_or(Error::Request)?
                    .expose_secret();
                if refresh.len() != 47 {
                    return Err(Error::Grant);
                }
                let hash = digest(refresh);
                let grant = sqlx::query_scalar!(
                    "SELECT grant_id FROM oauth_tokens WHERE token_hash=$1",
                    hash
                )
                .fetch_optional(&self.db.0)
                .await
                .map_err(DbError::from)?
                .ok_or(Error::Grant)?;
                self.access.grant(grant).await?;
                let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
                let grant = lock_grant(&mut tx, grant, &auth.client_id, &request.resource).await?;
                if !grant.refresh_allowed {
                    return Err(Error::Grant);
                }
                let consumed=sqlx::query_scalar!("SELECT consumed_at IS NOT NULL AS \"consumed!\" FROM oauth_tokens WHERE token_hash=$1",hash)
                    .fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::Grant)?;
                if consumed {
                    revoke_grant(&mut tx, grant.id).await?;
                    tx.commit().await.map_err(DbError::from)?;
                    return Err(Error::Grant);
                }
                sqlx::query!(
                    "UPDATE oauth_tokens SET consumed_at=now() WHERE token_hash=$1",
                    hash
                )
                .execute(&mut *tx)
                .await
                .map_err(DbError::from)?;
                sqlx::query!("UPDATE access_tokens SET revoked_at=COALESCE(revoked_at,now()) WHERE oauth_grant=$1",grant.id)
                    .execute(&mut *tx).await.map_err(DbError::from)?;
                let tokens = mint(&mut tx, &grant).await?;
                tx.commit().await.map_err(DbError::from)?;
                Ok(tokens)
            }
            _ => Err(Error::UnsupportedGrant),
        }
    }
}

async fn lock_grant(
    tx: &mut sqlx::PgConnection,
    id: Uuid,
    client: &str,
    resource: &str,
) -> Result<Grant, Error> {
    let hostname = resource_host(resource)?;
    sqlx::query!("SELECT e.id FROM endpoints e JOIN oauth_grants g ON g.endpoint_id=e.id WHERE g.id=$1 AND g.client_id=$2 AND g.scope=$3 FOR KEY SHARE OF e",id,client,resource_scope(resource)?)
        .fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::Grant)?;
    let row=sqlx::query!("SELECT id,client_id,scope,refresh_allowed,resource_url,expires_at>now() AND revoked_at IS NULL AS \"valid!\" FROM oauth_grants WHERE id=$1 FOR UPDATE",id)
        .fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::Grant)?;
    let canonical = canonical_resource(resource)?;
    let matching_resource = match row.resource_url {
        Some(saved) => saved == canonical,
        None => resource_workspace(resource).is_none(),
    };
    if !row.valid || !matching_resource {
        return Err(Error::Grant);
    }
    Ok(Grant {
        id: row.id,
        hostname,
        client_id: row.client_id,
        scope: row.scope,
        refresh_allowed: row.refresh_allowed,
    })
}
async fn mint(tx: &mut sqlx::PgConnection, grant: &Grant) -> Result<TokenResponse, Error> {
    // Older binaries must reject expiring OAuth tokens rather than treat them as permanent PATs.
    let access_token = random("cmo_");
    let name = format!("OAuth: {}", grant.client_id);
    let seconds=sqlx::query_scalar!("INSERT INTO access_tokens(name,audience,token_hash,expires_at,oauth_grant,tenant_id,workspace_id,resource) SELECT $1,$2,$3,LEAST(now()+interval '1 hour',g.expires_at),g.id,e.tenant_id,e.id,CASE WHEN g.scope='admin' THEN '/mcp-admin' ELSE '/mcp' END FROM oauth_grants g JOIN endpoints e ON e.id=g.endpoint_id WHERE g.id=$4 RETURNING extract(epoch FROM expires_at-now())::bigint AS \"seconds!\"",name,grant.hostname,digest(&access_token),grant.id)
        .fetch_one(&mut *tx).await.map_err(DbError::from)?;
    let refresh_token = if grant.refresh_allowed {
        let refresh = random("cmr_");
        sqlx::query!(
            "INSERT INTO oauth_tokens(token_hash,grant_id) VALUES($1,$2)",
            digest(&refresh),
            grant.id
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        Some(refresh)
    } else {
        None
    };
    Ok(TokenResponse {
        access_token,
        token_type: "Bearer",
        expires_in: seconds,
        refresh_token,
        scope: grant.scope.clone(),
    })
}
fn validate_authorization(request: &AuthorizationRequest) -> Result<(), Error> {
    if request.response_type != "code"
        || request.code_challenge_method != "S256"
        || request.code_challenge.len() != 43
        || !URL_SAFE_NO_PAD
            .decode(&request.code_challenge)
            .is_ok_and(|v| v.len() == 32)
        || request.state.as_ref().is_some_and(|s| s.len() > 2048)
    {
        return Err(Error::Request);
    }
    if request
        .scope
        .as_deref()
        .is_some_and(|s| !s.is_empty() && s != resource_scope(&request.resource).unwrap_or(""))
    {
        return Err(Error::Scope);
    }
    Ok(())
}
fn valid_verifier(value: &str) -> bool {
    (43..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
}
fn resource_host(value: &str) -> Result<String, Error> {
    if value.len() > 2048
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return Err(Error::Target);
    }
    let url = url::Url::parse(value).map_err(|_| Error::Target)?;
    if url.scheme() != "https"
        || url.port().is_some()
        || !matches!(url.path(), "/mcp" | "/mcp-admin")
        || url
            .query()
            .is_some_and(|query| url.path() != "/mcp" || parse_workspace_query(query).is_none())
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::Target);
    }
    url.host_str().map(str::to_owned).ok_or(Error::Target)
}

fn canonical_resource(value: &str) -> Result<String, Error> {
    resource_host(value)?;
    url::Url::parse(value)
        .map(|url| url.to_string())
        .map_err(|_| Error::Target)
}

fn resource_workspace(value: &str) -> Option<Uuid> {
    let url = url::Url::parse(value).ok()?;
    parse_workspace_query(url.query()?)
}

pub fn parse_workspace_query(query: &str) -> Option<Uuid> {
    let text = query.strip_prefix("workspace=")?;
    let id = Uuid::parse_str(text).ok()?;
    (id.hyphenated().to_string() == text).then_some(id)
}

fn resource_scope(value: &str) -> Result<&'static str, Error> {
    resource_host(value)?;
    let url = url::Url::parse(value).map_err(|_| Error::Target)?;
    Ok(if url.path() == "/mcp-admin" {
        "admin"
    } else {
        "mcp"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_resources_require_exactly_one_canonical_uuid() {
        let id = "abcdef01-2345-4678-9abc-def012345678";
        assert_eq!(
            parse_workspace_query(&format!("workspace={id}")),
            Uuid::parse_str(id).ok()
        );
        assert!(resource_host(&format!("https://work.example/mcp?workspace={id}")).is_ok());
        for query in [
            format!("workspace={id}&q=x"),
            format!("workspace={id}&workspace={id}"),
            format!("workspace={}", id.to_uppercase()),
            format!("workspace={id}&"),
        ] {
            assert!(parse_workspace_query(&query).is_none());
            assert!(resource_host(&format!("https://work.example/mcp?{query}")).is_err());
        }
        assert!(resource_host(&format!("https://work.example/mcp-admin?workspace={id}")).is_err());
    }
    #[test]
    fn pkce_and_resources_reject_ambiguous_inputs() {
        assert!(valid_verifier(&"a".repeat(43)));
        for value in ["a".repeat(42), "a".repeat(129), "%".repeat(43)] {
            assert!(!valid_verifier(&value));
        }
        assert_eq!(
            resource_host("HTTPS://WORK.EXAMPLE:443/mcp").unwrap(),
            "work.example"
        );
        for value in [
            "http://work.example/mcp",
            "https://work.example",
            "https://work.example/mcp/",
            "https://work.example:444/mcp",
            "https://work.example/mcp?q=x",
            "https://user@work.example/mcp",
            "https://work.example/mcp#fragment",
        ] {
            assert!(resource_host(value).is_err());
        }
    }
}
