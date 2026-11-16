use anyhow::{Context, ensure};
use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header, request::Parts},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use codemode_core::components::{
    auth::rate_limit::RateLimiter,
    sessions::{IdentityError, Sessions},
};
use idclient::{AppError, Application, Claims};
use secrecy::ExposeSecret;
use std::sync::Arc;

pub fn configuration(hostname: &str) -> anyhow::Result<Option<idclient::Config>> {
    let enabled: bool = std::env::var("CODEMODE_ID_ENABLED")
        .unwrap_or_else(|_| "false".into())
        .parse()
        .context("CODEMODE_ID_ENABLED must be true or false")?;
    if !enabled {
        return Ok(None);
    }
    let issuer =
        std::env::var("CODEMODE_ID_ISSUER").unwrap_or_else(|_| "https://id.microapps.space".into());
    let client =
        std::env::var("CODEMODE_ID_CLIENT_ID").context("CODEMODE_ID_CLIENT_ID is required")?;
    let secret = std::env::var("CODEMODE_ID_CLIENT_SECRET")
        .context("CODEMODE_ID_CLIENT_SECRET is required")?;
    let key =
        std::env::var("CODEMODE_ID_COOKIE_KEY").context("CODEMODE_ID_COOKIE_KEY is required")?;
    build_configuration(
        hostname,
        &issuer,
        &client,
        &secret,
        &key,
        std::env::var("CODEMODE_ENCRYPTION_KEY").ok().as_deref(),
    )
    .map(Some)
}

fn build_configuration(
    hostname: &str,
    issuer: &str,
    client: &str,
    secret: &str,
    key: &str,
    encryption_key: Option<&str>,
) -> anyhow::Result<idclient::Config> {
    let issuer_url = url::Url::parse(issuer).context("CODEMODE_ID_ISSUER must be an origin")?;
    let loopback = issuer_url.host_str().is_some_and(|host| {
        host.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    });
    ensure!(
        (issuer_url.scheme() == "https" || (issuer_url.scheme() == "http" && loopback))
            && issuer_url.username().is_empty()
            && issuer_url.password().is_none()
            && issuer_url.path() == "/"
            && issuer_url.query().is_none()
            && issuer_url.fragment().is_none(),
        "CODEMODE_ID_ISSUER must be an HTTPS origin or exact loopback HTTP origin"
    );
    ensure!(
        !client.is_empty()
            && client.len() <= 128
            && !client.chars().any(|c| c.is_control() || c.is_whitespace()),
        "CODEMODE_ID_CLIENT_ID is invalid"
    );
    ensure!(
        !secret.is_empty() && secret.len() <= 4096,
        "CODEMODE_ID_CLIENT_SECRET is invalid"
    );
    ensure!(
        key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "CODEMODE_ID_COOKIE_KEY must contain 64 hex characters"
    );
    ensure!(
        !encryption_key.is_some_and(|encryption| encryption.eq_ignore_ascii_case(key)),
        "CODEMODE_ID_COOKIE_KEY must be independent of CODEMODE_ENCRYPTION_KEY"
    );
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&key[index * 2..index * 2 + 2], 16)
            .context("invalid CODEMODE_ID_COOKIE_KEY")?;
    }
    ensure!(
        bytes.iter().any(|byte| *byte != 0),
        "CODEMODE_ID_COOKIE_KEY must be nonzero random key material"
    );
    Ok(idclient::Config {
        issuer: issuer_url.origin().ascii_serialization(),
        client_id: client.into(),
        client_secret: Some(secret.to_owned().into()),
        redirect_uri: format!("https://{hostname}/auth/callback"),
        cookie_key: Box::new(bytes).into(),
    })
}

pub struct App {
    sessions: Sessions,
}
impl App {
    pub fn new(sessions: Sessions) -> Self {
        Self { sessions }
    }
}
impl Application for App {
    type User = String;
    async fn resolve_user(&self, claims: Claims) -> Result<String, AppError> {
        if !claims.email_verified {
            return Err(AppError::Denied);
        }
        self.sessions
            .resolve_identity(&claims.subject, &claims.email)
            .await
            .map_err(resolve_error)?;
        Ok(claims.subject)
    }
    async fn create_session(
        &self,
        subject: String,
        request: Parts,
    ) -> Result<Vec<HeaderValue>, AppError> {
        let previous = crate::admin::cookie(&request.headers, true);
        let token = self
            .sessions
            .replace_identity_session(&subject, previous)
            .await
            .map_err(resolve_error)?
            .ok_or(AppError::Denied)?;
        let value = format!(
            "{}={}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=43200",
            crate::admin::ID_COOKIE,
            token.expose_secret()
        );
        Ok(vec![
            HeaderValue::from_str(&value).map_err(|_| AppError::Unavailable)?,
        ])
    }
    async fn clear_session(&self, request: Parts) -> Result<Vec<HeaderValue>, AppError> {
        if let Some(token) = crate::admin::cookie(&request.headers, true) {
            self.sessions
                .logout_identity(token)
                .await
                .map_err(resolve_error)?;
        }
        Ok(vec![HeaderValue::from_static(
            crate::admin::CLEAR_ID_COOKIE,
        )])
    }
}
fn resolve_error(error: IdentityError) -> AppError {
    match error {
        IdentityError::Database(error) => {
            tracing::error!(error=?error, "Microapps ID application session operation failed");
            AppError::Unavailable
        }
        _ => AppError::Denied,
    }
}

#[derive(Clone)]
struct Boundary {
    hostname: String,
    limit: Arc<RateLimiter>,
}
pub async fn router(
    config: idclient::Config,
    sessions: Sessions,
    hostname: String,
) -> anyhow::Result<Router> {
    let client = idclient::Client::new(config, App::new(sessions)).await?;
    Ok(client.routes().layer(middleware::from_fn_with_state(
        Boundary {
            hostname,
            limit: Arc::new(RateLimiter::new(60, 60)),
        },
        boundary,
    )))
}
async fn boundary(State(state): State<Boundary>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    let safe = matches!(*request.method(), Method::GET | Method::HEAD);
    let mut response = if headers.get_all(header::HOST).iter().count() != 1
        || !host
            .is_some_and(|host| host == state.hostname || host == format!("{}:443", state.hostname))
    {
        StatusCode::NOT_FOUND.into_response()
    } else if !safe
        && (headers.get_all(header::ORIGIN).iter().count() != 1
            || headers
                .get(header::ORIGIN)
                .and_then(|value| value.to_str().ok())
                != Some(format!("https://{}", state.hostname).as_str()))
    {
        StatusCode::FORBIDDEN.into_response()
    } else if let Err(seconds) = state.limit.check(uuid::Uuid::nil(), uuid::Uuid::nil()) {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, seconds.to_string())],
        )
            .into_response()
    } else {
        next.run(request).await
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_requires_independent_key_confidential_client_and_safe_origin() {
        let key = "31".repeat(32);
        let config = build_configuration(
            "test.example",
            "https://id.example/",
            "client",
            "secret",
            &key,
            None,
        )
        .unwrap();
        assert_eq!(config.redirect_uri, "https://test.example/auth/callback");
        assert_eq!(config.issuer, "https://id.example");
        for issuer in [
            "http://id.example",
            "https://user@id.example",
            "https://id.example/path",
            "https://id.example/?query=1",
            "https://id.example/#fragment",
        ] {
            assert!(
                build_configuration("test.example", issuer, "client", "secret", &key, None)
                    .is_err()
            );
        }
        for invalid in ["00".repeat(32), "gg".repeat(32), "1".repeat(63)] {
            assert!(
                build_configuration(
                    "test.example",
                    "https://id.example",
                    "client",
                    "secret",
                    &invalid,
                    None
                )
                .is_err()
            );
        }
        assert!(
            build_configuration(
                "test.example",
                "https://id.example",
                "client",
                "secret",
                &key,
                Some(&key)
            )
            .is_err()
        );
        assert!(
            build_configuration(
                "test.example",
                "https://id.example",
                "client",
                "",
                &key,
                None
            )
            .is_err()
        );
        assert!(
            build_configuration(
                "test.example",
                "https://id.example",
                "bad client",
                "secret",
                &key,
                None
            )
            .is_err()
        );
        assert!(
            build_configuration(
                "test.example",
                "http://127.0.0.1:12345",
                "client",
                "secret",
                &key,
                None
            )
            .is_ok()
        );
    }
}
