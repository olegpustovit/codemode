use axum::{
    Form, Json, Router,
    extract::{RawQuery, Request, State as AxumState, rejection::FormRejection},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use codemode_core::components::auth::oauth::{
    AuthMethod, ClientAuthentication, ClientMetadata, Error, OAuth, TokenRequest,
};
use secrecy::SecretString;
use std::sync::Arc;

#[derive(Clone)]
pub struct State {
    pub oauth: Arc<OAuth>,
    pub hostname: String,
}

pub fn router(state: State) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(metadata).options(preflight),
        )
        .route("/oauth/authorize", get(authorize))
        .route("/oauth/register", post(register).options(preflight))
        .route("/oauth/token", post(token).options(preflight))
        .route("/oauth/client-metadata.json", get(client_metadata))
        .route_layer(middleware::from_fn_with_state(state.clone(), boundary))
        .with_state(state)
}
async fn boundary(AxumState(state): AxumState<State>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .filter(|_| request.headers().get_all(header::HOST).iter().count() == 1)
        .and_then(codemode_core::components::workspaces::authority);
    let mut response = if host.as_deref() == Some(&state.hostname) {
        next.run(request).await
    } else {
        (StatusCode::NOT_FOUND, "unknown authorization server").into_response()
    };
    for (key, value) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::PRAGMA, "no-cache"),
        (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        (header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST, OPTIONS"),
        (
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            "Content-Type, Authorization",
        ),
        (header::REFERRER_POLICY, "no-referrer"),
    ] {
        response
            .headers_mut()
            .insert(key, axum::http::HeaderValue::from_static(value));
    }
    response
}
async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}
async fn client_metadata(AxumState(state): AxumState<State>) -> Json<serde_json::Value> {
    let origin = format!("https://{}", state.hostname);
    Json(
        serde_json::json!({"client_id":format!("{origin}/oauth/client-metadata.json"),"client_name":"Codemode Gateway","redirect_uris":[format!("{origin}/api/accounts/oauth/callback")],"token_endpoint_auth_method":"none","grant_types":["authorization_code","refresh_token"],"response_types":["code"],"application_type":"web"}),
    )
}
async fn metadata(AxumState(state): AxumState<State>) -> Json<serde_json::Value> {
    let issuer = format!("https://{}", state.hostname);
    Json(serde_json::json!({
        "issuer":issuer,"authorization_endpoint":format!("{issuer}/oauth/authorize"),
        "token_endpoint":format!("{issuer}/oauth/token"),"registration_endpoint":format!("{issuer}/oauth/register"),
        "response_types_supported":["code"],"response_modes_supported":["query"],
        "grant_types_supported":["authorization_code","refresh_token"],"scopes_supported":["mcp","admin"],
        "token_endpoint_auth_methods_supported":["none","client_secret_basic","client_secret_post"],
        "code_challenge_methods_supported":["S256"],"client_id_metadata_document_supported":true,
        "authorization_response_iss_parameter_supported":true,
    }))
}
async fn authorize(RawQuery(query): RawQuery) -> Response {
    let Some(query) = query.filter(|q| q.len() <= 8192) else {
        return failure(Error::Request);
    };
    match axum::http::HeaderValue::from_str(&format!("/authorize?{query}")) {
        Ok(location) => (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response(),
        Err(_) => failure(Error::Request),
    }
}
async fn register(
    AxumState(state): AxumState<State>,
    input: Result<Json<ClientMetadata>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let metadata = match input {
        Ok(Json(metadata)) => metadata,
        Err(_) => return invalid_metadata(),
    };
    match state.oauth.register(metadata).await {
        Ok(registration) => {
            use secrecy::ExposeSecret;
            let metadata = registration.metadata;
            let mut response = serde_json::json!({"client_id":registration.client_id,
                "client_name":metadata.client_name,"redirect_uris":metadata.redirect_uris,
                "token_endpoint_auth_method":metadata.token_endpoint_auth_method,
                "grant_types":metadata.grant_types,"response_types":metadata.response_types});
            if let Some(secret) = registration.client_secret {
                response["client_secret"] =
                    serde_json::Value::String(secret.expose_secret().to_owned());
                response["client_secret_expires_at"] = serde_json::json!(0);
            }
            (StatusCode::CREATED, Json(response)).into_response()
        }
        Err(Error::Client) => invalid_metadata(),
        Err(error) => failure(error),
    }
}
fn invalid_metadata() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error":"invalid_client_metadata"})),
    )
        .into_response()
}
#[derive(serde::Deserialize)]
struct TokenForm {
    client_id: Option<String>,
    client_secret: Option<SecretString>,
    #[serde(flatten)]
    request: TokenRequest,
}
async fn token(
    AxumState(state): AxumState<State>,
    headers: axum::http::HeaderMap,
    input: Result<Form<TokenForm>, FormRejection>,
) -> Response {
    let form = match input {
        Ok(Form(form)) => form,
        Err(_) => return failure(Error::Request),
    };
    let auth = match authentication(&headers, form.client_id, form.client_secret) {
        Ok(auth) => auth,
        Err(error) => return failure(error),
    };
    match state.oauth.token(&auth, &form.request).await {
        Ok(tokens) => Json(tokens).into_response(),
        Err(error) => failure(error),
    }
}
fn authentication(
    headers: &axum::http::HeaderMap,
    client_id: Option<String>,
    client_secret: Option<SecretString>,
) -> Result<ClientAuthentication, Error> {
    if headers.get_all(header::AUTHORIZATION).iter().count() > 1 {
        return Err(Error::Client);
    }
    if let Some(header) = headers.get(header::AUTHORIZATION) {
        let value = header.to_str().map_err(|_| Error::Client)?;
        if value.len() > 8192 || client_secret.is_some() {
            return Err(Error::Client);
        }
        let (scheme, encoded) = value.split_once(' ').ok_or(Error::Client)?;
        if !scheme.eq_ignore_ascii_case("basic") {
            return Err(Error::Client);
        }
        let bytes = STANDARD.decode(encoded).map_err(|_| Error::Client)?;
        let credentials = std::str::from_utf8(&bytes).map_err(|_| Error::Client)?;
        let (id, secret) = credentials.split_once(':').ok_or(Error::Client)?;
        let id = decode_component(id)?;
        let secret = decode_component(secret)?;
        if client_id.is_some_and(|client| client != id) {
            return Err(Error::Client);
        }
        return Ok(ClientAuthentication {
            client_id: id,
            method: AuthMethod::ClientSecretBasic,
            secret: Some(secret.into()),
        });
    }
    Ok(ClientAuthentication {
        client_id: client_id.ok_or(Error::Client)?,
        method: if client_secret.is_some() {
            AuthMethod::ClientSecretPost
        } else {
            AuthMethod::None
        },
        secret: client_secret,
    })
}
fn decode_component(value: &str) -> Result<String, Error> {
    let encoded = format!("v={value}");
    let mut pairs = url::form_urlencoded::parse(encoded.as_bytes());
    let (_, value) = pairs.next().ok_or(Error::Client)?;
    if pairs.next().is_some() {
        return Err(Error::Client);
    }
    Ok(value.into_owned())
}
fn failure(error: Error) -> Response {
    let (status, code) = match error {
        Error::Access(codemode_core::components::access::Error::Denied) => {
            (StatusCode::FORBIDDEN, "access_denied")
        }
        Error::Access(codemode_core::components::access::Error::Unavailable) => {
            (StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable")
        }
        Error::Database(error) => {
            tracing::error!(error=?error,"OAuth operation failed");
            (StatusCode::SERVICE_UNAVAILABLE, "server_error")
        }
        Error::Busy => (StatusCode::TOO_MANY_REQUESTS, "temporarily_unavailable"),
        Error::Client => (StatusCode::UNAUTHORIZED, "invalid_client"),
        Error::Grant => (StatusCode::BAD_REQUEST, "invalid_grant"),
        Error::Request => (StatusCode::BAD_REQUEST, "invalid_request"),
        Error::UnsupportedGrant => (StatusCode::BAD_REQUEST, "unsupported_grant_type"),
        Error::Scope => (StatusCode::BAD_REQUEST, "invalid_scope"),
        Error::Target => (StatusCode::BAD_REQUEST, "invalid_target"),
    };
    let mut response = (status, Json(serde_json::json!({"error":code}))).into_response();
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            axum::http::HeaderValue::from_static("Basic realm=\"codemode\""),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    #[test]
    fn client_authentication_enforces_one_method_and_decodes_basic_components() {
        let mut headers = axum::http::HeaderMap::new();
        let none = authentication(&headers, Some("client".into()), None).unwrap();
        assert_eq!(none.method, AuthMethod::None);
        let post = authentication(&headers, Some("client".into()), Some("secret".into())).unwrap();
        assert_eq!(post.method, AuthMethod::ClientSecretPost);
        let basic = format!("Basic {}", STANDARD.encode("client%3Aid:secret%2Bvalue"));
        headers.insert(header::AUTHORIZATION, basic.parse().unwrap());
        let basic = authentication(&headers, None, None).unwrap();
        assert_eq!(basic.client_id, "client:id");
        assert_eq!(basic.method, AuthMethod::ClientSecretBasic);
        assert_eq!(basic.secret.unwrap().expose_secret(), "secret+value");
        assert!(authentication(&headers, Some("different".into()), None).is_err());
        assert!(authentication(&headers, None, Some("second method".into())).is_err());
        headers.append(header::AUTHORIZATION, "Basic aWQ6c2VjcmV0".parse().unwrap());
        assert!(authentication(&headers, None, None).is_err());
    }
}
