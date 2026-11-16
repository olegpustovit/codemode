use axum::{
    Json, Router,
    extract::{FromRequest, Path, Query, Request, State as AxumState},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use codemode_core::components::{
    accounts::{self, Accounts},
    agent_sessions::{self, AgentSessions},
    auth::Auth,
    bulk,
    execution::log,
    integrations, policies,
    scripts::{self, Scripts},
    sessions::{self, Sessions},
    sources::{self, Sources},
    workspaces::{self, Workspaces},
};
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;
use uuid::Uuid;

const COOKIE: &str = "__Host-codemode_session";
const CLEAR_COOKIE: &str =
    "__Host-codemode_session=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0";

#[derive(Clone)]
pub struct State {
    pub scripts: Scripts,
    pub log: log::Log,
    pub agent_sessions: AgentSessions,
    pub auth: Auth,
    pub sessions: Arc<Sessions>,
    pub identity_enabled: bool,
    pub accounts: Arc<Accounts>,
    pub sources: Arc<Sources>,
    pub hostname: String,
    pub workspaces: Arc<Workspaces>,
    pub oauth: Arc<codemode_core::components::auth::oauth::OAuth>,
}

pub fn router(state: State) -> Router {
    let protected = Router::new()
        .route("/api/workspaces/{id}/policies/bulk", post(bulk_policies))
        .route(
            "/api/workspaces/{id}/scripts/bulk-delete",
            post(bulk_scripts),
        )
        .route("/api/tokens/bulk-revoke", post(bulk_tokens))
        .route("/api/sources/bulk-sync", post(bulk_sources))
        .route(
            "/api/oauth/authorize",
            get(oauth_consent).post(oauth_decision),
        )
        .route("/api/workspaces", get(endpoints).post(create_endpoint))
        .route(
            "/api/workspaces/{id}",
            get(endpoint).put(update_endpoint).delete(remove_endpoint),
        )
        .route(
            "/api/workspaces/{id}/policies",
            get(tool_policies).put(set_tool_policy),
        )
        .route(
            "/api/workspaces/{id}/policies/source",
            put(set_source_policy),
        )
        .route("/api/workspaces/{id}/scope", put(set_tool_scope_mode))
        .route(
            "/api/workspaces/{id}/scope/tools",
            put(set_tool_scope_member),
        )
        .route(
            "/api/workspaces/{id}/scripts",
            get(saved_scripts).post(create_script),
        )
        .route(
            "/api/workspaces/{id}/scripts/{script}",
            get(saved_script).put(update_script).delete(remove_script),
        )
        .route("/api/integrations", get(integration_list))
        .route("/api/integrations/{slug}", post(add_integration))
        .route("/api/sources", get(sources).post(create_source))
        .route(
            "/api/sources/{id}",
            get(source)
                .put(update_source)
                .patch(update_source_display)
                .delete(remove_source),
        )
        .route("/api/sources/{id}/sync", axum::routing::post(sync_source))
        .route("/api/tokens", get(tokens).post(create_token))
        .route("/api/tokens/{id}", delete(revoke_token))
        .route("/api/accounts", get(accounts))
        .route("/api/oauth-apps", get(configured_oauth_apps))
        .route(
            "/api/oauth-apps/{provider}",
            put(save_oauth_app).delete(remove_oauth_app),
        )
        .route(
            "/api/accounts/{id}/oauth",
            axum::routing::post(start_upstream_oauth),
        )
        .route("/api/accounts/oauth/callback", get(upstream_oauth_callback))
        .route(
            "/api/accounts/{id}",
            put(link_account).delete(unlink_account),
        )
        .route("/api/accounts/{id}/environment", put(link_environment))
        .route("/api/accounts/{id}/http", put(link_http_account))
        .route("/api/sessions", get(agent_session_list))
        .route("/api/session-activity", get(agent_session_activity))
        .route(
            "/api/sessions/{id}",
            get(agent_session_detail).patch(rename_agent_session),
        )
        .route("/api/executions", get(executions))
        .route("/api/executions/{id}", get(execution))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_session,
        ));
    Router::new()
        .route("/api/config", get(public_config))
        .route("/api/session", get(session).post(login).delete(logout))
        .route("/api/register", post(register))
        .merge(protected)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            browser_boundary,
        ))
        .with_state(state)
}

enum ApiError {
    Message(StatusCode, &'static str),
    OAuth {
        stage: accounts::OAuthStage,
        detail: String,
    },
}
struct ApiJson<T>(T);
impl<S: Send + Sync, T: serde::de::DeserializeOwned> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;
    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|error| ApiError::Message(error.status(), "invalid JSON request"))
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Message(status, message) => (status, Json(serde_json::json!({"error":message}))).into_response(),
            Self::OAuth { stage, detail } => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({
                "error":format!("upstream authorization failed at {stage}: {detail}; {}", stage.recovery()),
                "stage":stage, "detail":detail,
            }))).into_response(),
        }
    }
}
fn unavailable(error: impl std::fmt::Debug) -> ApiError {
    tracing::error!(error=?error,"control-plane operation failed");
    ApiError::Message(StatusCode::SERVICE_UNAVAILABLE, "operation unavailable")
}
impl From<sessions::Error> for ApiError {
    fn from(error: sessions::Error) -> Self {
        match error {
            sessions::Error::Busy => Self::Message(
                StatusCode::TOO_MANY_REQUESTS,
                "too many sign-in attempts; try again in one minute",
            ),
            sessions::Error::NotAllowed => Self::Message(
                StatusCode::FORBIDDEN,
                "registration is limited to ursus95@gmail.com and @olegpustovit.com addresses",
            ),
            sessions::Error::Exists => Self::Message(
                StatusCode::CONFLICT,
                "email is already registered; sign in instead",
            ),
            sessions::Error::Configuration => Self::Message(
                StatusCode::BAD_REQUEST,
                "password must contain 12..1024 bytes",
            ),
            other => unavailable(other),
        }
    }
}
impl From<accounts::Error> for ApiError {
    fn from(error: accounts::Error) -> Self {
        match error {
            accounts::Error::Invalid => {
                Self::Message(StatusCode::BAD_REQUEST, "invalid credential")
            }
            accounts::Error::NotFound => Self::Message(StatusCode::NOT_FOUND, "source not found"),
            accounts::Error::Provider => Self::Message(
                StatusCode::BAD_REQUEST,
                "provider does not support OAuth-app configuration",
            ),
            accounts::Error::OAuth { stage, detail } => {
                tracing::warn!(%stage, %detail, "upstream authorization failed");
                Self::OAuth { stage, detail }
            }
            accounts::Error::State => Self::Message(
                StatusCode::BAD_REQUEST,
                "invalid or expired authorization callback",
            ),
            accounts::Error::Changed => Self::Message(
                StatusCode::CONFLICT,
                "source or account changed; retry linking the account",
            ),
            accounts::Error::Busy => Self::Message(
                StatusCode::TOO_MANY_REQUESTS,
                "account authorization is busy; retry shortly",
            ),
            other => unavailable(other),
        }
    }
}
impl From<sources::Error> for ApiError {
    fn from(error: sources::Error) -> Self {
        match error {
            sources::Error::Display(message) => Self::Message(StatusCode::BAD_REQUEST, message),
            sources::Error::Busy => Self::Message(
                StatusCode::TOO_MANY_REQUESTS,
                "OpenAPI document loading is busy; retry shortly",
            ),
            sources::Error::NotFound => Self::Message(StatusCode::NOT_FOUND, "source not found"),
            sources::Error::Conflict => {
                Self::Message(StatusCode::CONFLICT, "source slug is already in use")
            }
            sources::Error::OpenApi(
                codemode_core::components::openapi::Error::Invalid(reason)
                | codemode_core::components::openapi::Error::Arguments(reason),
            ) => Self::Message(StatusCode::BAD_REQUEST, reason),
            sources::Error::OpenApi(_) => Self::Message(
                StatusCode::BAD_GATEWAY,
                "OpenAPI document could not be fetched; check its address and ensure it is accessible without redirects",
            ),
            sources::Error::Config(_) => Self::Message(
                StatusCode::BAD_REQUEST,
                "invalid source configuration; use an HTTP(S) MCP URL without credentials or an absolute stdio command",
            ),
            other => unavailable(other),
        }
    }
}
impl From<workspaces::Error> for ApiError {
    fn from(error: workspaces::Error) -> Self {
        match error {
            workspaces::Error::Invalid => Self::Message(
                StatusCode::BAD_REQUEST,
                "workspace name must contain 1..80 characters",
            ),
            workspaces::Error::Reserved => Self::Message(
                StatusCode::CONFLICT,
                "the default workspace cannot be removed or have its sources changed",
            ),
            workspaces::Error::Conflict => {
                Self::Message(StatusCode::CONFLICT, "workspace name already exists")
            }
            workspaces::Error::NotFound => {
                Self::Message(StatusCode::NOT_FOUND, "workspace not found")
            }
            workspaces::Error::Sources => Self::Message(
                StatusCode::BAD_REQUEST,
                "workspace contains unknown or duplicate sources",
            ),
            workspaces::Error::Limit => {
                Self::Message(StatusCode::BAD_REQUEST, "maximum 64 custom workspaces")
            }
            other => unavailable(other),
        }
    }
}

impl From<codemode_core::components::auth::oauth::Error> for ApiError {
    fn from(error: codemode_core::components::auth::oauth::Error) -> Self {
        use codemode_core::components::auth::oauth::Error;
        match error {
            Error::Database(error) => unavailable(error),
            Error::Busy => Self::Message(
                StatusCode::TOO_MANY_REQUESTS,
                "OAuth operation busy; try again later",
            ),
            Error::Client => Self::Message(StatusCode::BAD_REQUEST, "invalid_client"),
            Error::Grant => Self::Message(StatusCode::BAD_REQUEST, "invalid_grant"),
            Error::Scope => Self::Message(StatusCode::BAD_REQUEST, "invalid_scope"),
            Error::Target => Self::Message(StatusCode::BAD_REQUEST, "invalid_target"),
            Error::Request => Self::Message(StatusCode::BAD_REQUEST, "invalid_request"),
            Error::UnsupportedGrant => {
                Self::Message(StatusCode::BAD_REQUEST, "unsupported_grant_type")
            }
        }
    }
}
async fn oauth_consent(
    AxumState(state): AxumState<State>,
    Query(request): Query<codemode_core::components::auth::oauth::AuthorizationRequest>,
) -> Result<Json<codemode_core::components::auth::oauth::Consent>, ApiError> {
    state.oauth.prepare_client(&request.client_id).await?;
    Ok(Json(state.oauth.consent(&request).await?))
}
#[derive(serde::Deserialize)]
struct OAuthDecision {
    #[serde(flatten)]
    request: codemode_core::components::auth::oauth::AuthorizationRequest,
    allow: bool,
}
async fn oauth_decision(
    AxumState(state): AxumState<State>,
    ApiJson(decision): ApiJson<OAuthDecision>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .oauth
        .prepare_client(&decision.request.client_id)
        .await?;
    state.oauth.consent(&decision.request).await?;
    let code = if decision.allow {
        Some(state.oauth.issue_code(&decision.request).await?)
    } else {
        None
    };
    let mut redirect = url::Url::parse(&decision.request.redirect_uri)
        .map_err(|_| ApiError::Message(StatusCode::BAD_REQUEST, "invalid_request"))?;
    {
        let mut query = redirect.query_pairs_mut();
        if let Some(code) = code {
            query.append_pair("code", &code);
        } else {
            query.append_pair("error", "access_denied");
        }
        if let Some(client_state) = &decision.request.state {
            query.append_pair("state", client_state);
        }
        query.append_pair("iss", &format!("https://{}", state.hostname));
    }
    Ok(Json(serde_json::json!({"redirectUri":redirect.as_str()})))
}
async fn endpoints(
    AxumState(state): AxumState<State>,
) -> Result<Json<Vec<workspaces::Workspace>>, ApiError> {
    Ok(Json(state.workspaces.list().await?))
}
async fn endpoint(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
) -> Result<Json<workspaces::Workspace>, ApiError> {
    state
        .workspaces
        .get(id)
        .await?
        .map(Json)
        .ok_or(ApiError::Message(
            StatusCode::NOT_FOUND,
            "workspace not found",
        ))
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NewEndpoint {
    name: String,
    source_ids: Vec<Uuid>,
}
async fn create_endpoint(
    AxumState(state): AxumState<State>,
    ApiJson(input): ApiJson<NewEndpoint>,
) -> Result<Response, ApiError> {
    let id = state
        .workspaces
        .create(&input.name, &input.source_ids)
        .await?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({"id":id}))).into_response())
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EndpointSources {
    name: String,
    source_ids: Vec<Uuid>,
}
async fn update_endpoint(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(input): ApiJson<EndpointSources>,
) -> Result<StatusCode, ApiError> {
    state
        .workspaces
        .update(id, &input.name, &input.source_ids)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn remove_endpoint(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if state.workspaces.remove(id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::Message(
            StatusCode::NOT_FOUND,
            "workspace not found",
        ))
    }
}

async fn sources(
    AxumState(state): AxumState<State>,
) -> Result<Json<Vec<sources::Record>>, ApiError> {
    Ok(Json(state.sources.list().await?))
}
async fn source(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
) -> Result<Json<sources::Record>, ApiError> {
    state
        .sources
        .get(id)
        .await?
        .map(Json)
        .ok_or(ApiError::Message(StatusCode::NOT_FOUND, "source not found"))
}
pub(crate) fn user_source(source: &sources::Source) -> Result<(), &'static str> {
    if matches!(source.transport, sources::Transport::McpStdio { .. }) {
        return Err("local MCP commands must be installed by the server operator");
    }
    Ok(())
}

async fn create_source(
    AxumState(state): AxumState<State>,
    ApiJson(input): ApiJson<sources::SourceInput>,
) -> Result<Response, ApiError> {
    user_source(&input.source)
        .map_err(|message| ApiError::Message(StatusCode::FORBIDDEN, message))?;
    let id = state.sources.save_input(None, input).await?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({"id":id}))).into_response())
}
async fn update_source(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(input): ApiJson<sources::SourceInput>,
) -> Result<StatusCode, ApiError> {
    user_source(&input.source)
        .map_err(|message| ApiError::Message(StatusCode::FORBIDDEN, message))?;
    state.sources.save_input(Some(id), input).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn update_source_display(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(input): ApiJson<sources::Display>,
) -> Result<StatusCode, ApiError> {
    state.sources.set_display(id, input).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn remove_source(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if state.sources.remove(id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::Message(StatusCode::NOT_FOUND, "source not found"))
    }
}
async fn sync_source(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.sources.resync(id).await?;
    Ok(StatusCode::ACCEPTED)
}

pub(crate) const ID_COOKIE: &str = "__Host-codemode_id_session";
pub(crate) const CLEAR_ID_COOKIE: &str =
    "__Host-codemode_id_session=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0";

pub(crate) fn cookie(headers: &HeaderMap, identity_enabled: bool) -> Option<&str> {
    let expected = if identity_enabled { ID_COOKIE } else { COOKIE };
    let mut found = None;
    for line in headers.get_all(header::COOKIE) {
        for part in line.to_str().ok()?.split(';') {
            if let Some((name, value)) = part.trim().split_once('=')
                && name == expected
            {
                if found.is_some() {
                    return None;
                }
                found = Some(value);
            }
        }
    }
    found
}

async fn browser_boundary(
    AxumState(state): AxumState<State>,
    request: Request,
    next: Next,
) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let safe_method = matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD
    );
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    let same_origin = request.headers().get_all(header::ORIGIN).iter().count() == 1
        && origin == Some(format!("https://{}", state.hostname).as_str());
    let mut response = if request.headers().get_all(header::HOST).iter().count() != 1
        || (host != state.hostname && host != format!("{}:443", state.hostname))
    {
        ApiError::Message(StatusCode::NOT_FOUND, "unknown control plane").into_response()
    } else if !safe_method && !same_origin {
        ApiError::Message(
            StatusCode::FORBIDDEN,
            "same-origin browser request required",
        )
        .into_response()
    } else {
        next.run(request).await
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("no-referrer"),
    );
    response
}

async fn require_session(
    AxumState(state): AxumState<State>,
    request: Request,
    next: Next,
) -> Response {
    let Some(token) = cookie(request.headers(), state.identity_enabled) else {
        return ApiError::Message(StatusCode::UNAUTHORIZED, "sign in required").into_response();
    };
    match browser_user(&state.sessions, state.identity_enabled, token).await {
        Ok(Some(_)) => next.run(request).await,
        Ok(None) => ApiError::Message(StatusCode::UNAUTHORIZED, "sign in required").into_response(),
        Err(error) => unavailable(error).into_response(),
    }
}

async fn public_config(AxumState(state): AxumState<State>) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"name":"Codemode","hostname":state.hostname,"loginEnabled":true,"idEnabled":state.identity_enabled}),
    )
}
async fn session(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = match cookie(&headers, state.identity_enabled) {
        Some(token) => browser_user(&state.sessions, state.identity_enabled, token)
            .await
            .map_err(unavailable)?,
        None => None,
    };
    Ok(Json(
        serde_json::json!({"authenticated":user.is_some(),"user":user,"firstUser":!state.identity_enabled && state.sessions.first_user().await?}),
    ))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    email: String,
    password: SecretString,
}
async fn login(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<Login>,
) -> Result<Response, ApiError> {
    if state.identity_enabled {
        return Err(ApiError::Message(
            StatusCode::FORBIDDEN,
            "sign in with ID required",
        ));
    }
    let token = state
        .sessions
        .login(&input.email, input.password)
        .await?
        .ok_or(ApiError::Message(
            StatusCode::UNAUTHORIZED,
            "incorrect email or password",
        ))?;
    signed_in(&state, &headers, token).await
}
async fn register(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<Login>,
) -> Result<Response, ApiError> {
    if state.identity_enabled {
        return Err(ApiError::Message(
            StatusCode::FORBIDDEN,
            "sign in with ID required",
        ));
    }
    let token = state
        .sessions
        .register(&input.email, input.password)
        .await?;
    signed_in(&state, &headers, token).await
}
async fn signed_in(
    state: &State,
    headers: &HeaderMap,
    token: SecretString,
) -> Result<Response, ApiError> {
    if let Some(previous) = cookie(headers, state.identity_enabled) {
        state.sessions.logout(previous).await?;
    }
    let user = state.sessions.user(token.expose_secret()).await?;
    let value = format!(
        "{COOKIE}={}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=43200",
        token.expose_secret()
    );
    Ok((
        [(header::SET_COOKIE, value)],
        Json(serde_json::json!({"authenticated":true,"user":user,"firstUser":false})),
    )
        .into_response())
}
async fn logout(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(token) = cookie(&headers, state.identity_enabled) {
        if state.identity_enabled {
            state
                .sessions
                .logout_identity(token)
                .await
                .map_err(unavailable)?;
        } else {
            state.sessions.logout(token).await?;
        }
    }
    let clear = if state.identity_enabled {
        CLEAR_ID_COOKIE
    } else {
        CLEAR_COOKIE
    };
    Ok(([(header::SET_COOKIE, clear)], StatusCode::NO_CONTENT).into_response())
}

async fn tokens(
    AxumState(state): AxumState<State>,
) -> Result<Json<Vec<codemode_core::components::auth::Token>>, ApiError> {
    Ok(Json(state.auth.list().await.map_err(unavailable)?))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NewToken {
    name: String,
    #[serde(rename = "workspaceId")]
    workspace_id: Option<Uuid>,
    #[serde(default)]
    administration: bool,
}
async fn create_token(
    AxumState(state): AxumState<State>,
    ApiJson(input): ApiJson<NewToken>,
) -> Result<Response, ApiError> {
    let name = input.name.trim();
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err(ApiError::Message(
            StatusCode::BAD_REQUEST,
            "name must contain 1..128 bytes",
        ));
    }
    let workspace_id = if input.administration {
        state.workspaces.default_id().await?
    } else {
        input.workspace_id.ok_or(ApiError::Message(
            StatusCode::BAD_REQUEST,
            "choose a workspace",
        ))?
    };
    let (id, token) = state
        .auth
        .create_for_workspace(
            name,
            workspace_id,
            &state.hostname,
            if input.administration {
                "/mcp-admin"
            } else {
                "/mcp"
            },
        )
        .await
        .map_err(unavailable)?
        .ok_or(ApiError::Message(
            StatusCode::NOT_FOUND,
            "workspace not found",
        ))?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({"id":id,"token":token})),
    )
        .into_response())
}
async fn revoke_token(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if state.auth.revoke(id).await.map_err(unavailable)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::Message(StatusCode::NOT_FOUND, "token not found"))
    }
}
async fn accounts(
    AxumState(state): AxumState<State>,
) -> Result<Json<Vec<accounts::Account>>, ApiError> {
    Ok(Json(state.accounts.list().await?))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Credential {
    credential: SecretString,
}
async fn link_http_account(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(input): ApiJson<accounts::HttpCredential>,
) -> Result<StatusCode, ApiError> {
    state.accounts.link_http(id, input).await?;
    state.sources.wake();
    Ok(StatusCode::NO_CONTENT)
}
async fn link_account(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(input): ApiJson<Credential>,
) -> Result<StatusCode, ApiError> {
    state.accounts.link_bearer(id, input.credential).await?;
    state.sources.wake();
    Ok(StatusCode::NO_CONTENT)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LogQuery {
    before: Option<Uuid>,
}
async fn executions(
    AxumState(state): AxumState<State>,
    Query(query): Query<LogQuery>,
) -> Result<Json<Vec<log::Summary>>, ApiError> {
    Ok(Json(
        state.log.list(query.before).await.map_err(unavailable)?,
    ))
}
async fn execution(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
) -> Result<Json<log::Detail>, ApiError> {
    state
        .log
        .detail(id)
        .await
        .map_err(unavailable)?
        .map(Json)
        .ok_or(ApiError::Message(
            StatusCode::NOT_FOUND,
            "execution not found",
        ))
}
async fn unlink_account(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.accounts.unlink(id).await?;
    state.sources.wake();
    Ok(StatusCode::NO_CONTENT)
}

async fn start_upstream_oauth(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = cookie(&headers, state.identity_enabled).ok_or(ApiError::Message(
        StatusCode::UNAUTHORIZED,
        "sign in required",
    ))?;
    let url = state
        .accounts
        .start_oauth(id, &state.hostname, session)
        .await?;
    Ok(Json(serde_json::json!({"authorizationUrl":url})))
}
#[derive(serde::Deserialize)]
struct UpstreamCallback {
    state: String,
    code: Option<SecretString>,
    iss: Option<String>,
    error: Option<SecretString>,
    error_description: Option<SecretString>,
}
async fn upstream_oauth_callback(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
    Query(callback): Query<UpstreamCallback>,
) -> Result<axum::response::Redirect, ApiError> {
    let session = cookie(&headers, state.identity_enabled).ok_or(ApiError::Message(
        StatusCode::UNAUTHORIZED,
        "sign in required",
    ))?;
    let denial = callback.error.as_ref().map(|error| {
        format!(
            "{}: {}",
            error.expose_secret(),
            callback
                .error_description
                .as_ref()
                .map(ExposeSecret::expose_secret)
                .unwrap_or("authorization was declined")
        )
    });
    let result = state
        .accounts
        .complete_oauth(
            &callback.state,
            session,
            callback.code.as_ref().map(ExposeSecret::expose_secret),
            callback.iss.as_deref(),
            denial.as_deref(),
        )
        .await?;
    let (id, status, detail) = match result {
        accounts::OAuthCompletion::Linked(id) => {
            state.sources.wake();
            (id, "linked", None)
        }
        accounts::OAuthCompletion::Denied(id, detail) => (id, "denied", Some(detail)),
    };
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.extend_pairs([
        ("selected", id.to_string().as_str()),
        ("detail", "open"),
        ("oauth", status),
    ]);
    if let Some(detail) = detail {
        query.append_pair("oauthError", &detail);
    }
    Ok(axum::response::Redirect::to(&format!(
        "/integrations?{}",
        query.finish()
    )))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Environment {
    environment: std::collections::BTreeMap<String, SecretString>,
}
async fn link_environment(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(input): ApiJson<Environment>,
) -> Result<StatusCode, ApiError> {
    state
        .accounts
        .link_environment(id, input.environment)
        .await?;
    state.sources.wake();
    Ok(StatusCode::NO_CONTENT)
}

impl From<policies::Error> for ApiError {
    fn from(error: policies::Error) -> Self {
        match error {
            policies::Error::NotFound => Self::Message(
                StatusCode::NOT_FOUND,
                "endpoint or tool changed; reload before continuing",
            ),
            policies::Error::Invalid => {
                Self::Message(StatusCode::BAD_REQUEST, "invalid policy request")
            }
            other => unavailable(other),
        }
    }
}
#[derive(serde::Deserialize)]
struct PolicyQuery {
    #[serde(default)]
    q: String,
    cursor: Option<String>,
}
async fn tool_policies(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    Query(query): Query<PolicyQuery>,
) -> Result<Json<policies::Page>, ApiError> {
    state.workspaces.get(id).await?.ok_or(ApiError::Message(
        StatusCode::NOT_FOUND,
        "workspace not found",
    ))?;
    Ok(Json(
        state
            .sources
            .policies()
            .list(id, &query.q, query.cursor.as_deref())
            .await?,
    ))
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PolicyRequest {
    source_id: Uuid,
    source_revision: i64,
    upstream_name: String,
    action: policies::Choice,
}
async fn set_tool_policy(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(request): ApiJson<PolicyRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .sources
        .policies()
        .set(
            id,
            request.source_id,
            request.source_revision,
            &request.upstream_name,
            request.action,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourcePolicyRequest {
    source_id: Uuid,
    source_revision: i64,
    action: policies::Action,
}
async fn set_source_policy(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(request): ApiJson<SourcePolicyRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .sources
        .policies()
        .set_source(
            id,
            request.source_id,
            request.source_revision,
            request.action,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolScopeMode {
    all_tools: bool,
}
async fn set_tool_scope_mode(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(request): ApiJson<ToolScopeMode>,
) -> Result<StatusCode, ApiError> {
    state
        .workspaces
        .set_all_tools(id, request.all_tools)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolScopeMember {
    source_id: Uuid,
    source_revision: i64,
    upstream_name: String,
    included: bool,
}
async fn set_tool_scope_member(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(request): ApiJson<ToolScopeMember>,
) -> Result<StatusCode, ApiError> {
    state
        .sources
        .policies()
        .set_in_scope(
            id,
            request.source_id,
            request.source_revision,
            &request.upstream_name,
            request.included,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

impl From<scripts::Error> for ApiError {
    fn from(error: scripts::Error) -> Self {
        match error {
            scripts::Error::NotFound => {
                Self::Message(StatusCode::NOT_FOUND, "saved script or endpoint not found")
            }
            scripts::Error::Invalid => Self::Message(
                StatusCode::BAD_REQUEST,
                "name must contain 1..120 characters, description at most 2000, and Python source at most 64 KiB",
            ),
            scripts::Error::Conflict => Self::Message(
                StatusCode::CONFLICT,
                "a script with this name already exists in the endpoint",
            ),
            scripts::Error::Changed => Self::Message(
                StatusCode::CONFLICT,
                "script changed; copy your edits before reloading the latest version",
            ),
            scripts::Error::Limit => Self::Message(
                StatusCode::CONFLICT,
                "maximum 128 saved scripts per endpoint",
            ),
            other => unavailable(other),
        }
    }
}
async fn saved_scripts(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<scripts::Summary>>, ApiError> {
    state
        .workspaces
        .get(id)
        .await?
        .ok_or(workspaces::Error::NotFound)?;
    Ok(Json(state.scripts.list(id).await?))
}
async fn saved_script(
    AxumState(state): AxumState<State>,
    Path((endpoint, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<scripts::Script>, ApiError> {
    Ok(Json(state.scripts.get(endpoint, id).await?))
}
async fn create_script(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(content): ApiJson<scripts::Content>,
) -> Result<(StatusCode, Json<scripts::Script>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(state.scripts.save(id, None, content).await?),
    ))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptUpdate {
    revision: i64,
    name: String,
    description: String,
    code: String,
}
async fn update_script(
    AxumState(state): AxumState<State>,
    Path((endpoint, id)): Path<(Uuid, Uuid)>,
    ApiJson(update): ApiJson<ScriptUpdate>,
) -> Result<Json<scripts::Script>, ApiError> {
    Ok(Json(
        state
            .scripts
            .save(
                endpoint,
                Some((id, update.revision)),
                scripts::Content {
                    name: update.name,
                    description: update.description,
                    code: update.code,
                },
            )
            .await?,
    ))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptRevision {
    revision: i64,
}
async fn remove_script(
    AxumState(state): AxumState<State>,
    Path((endpoint, id)): Path<(Uuid, Uuid)>,
    ApiJson(request): ApiJson<ScriptRevision>,
) -> Result<StatusCode, ApiError> {
    state.scripts.remove(endpoint, id, request.revision).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod oauth_error_tests {
    use super::*;
    #[tokio::test]
    async fn oauth_error_response_identifies_the_stage_and_recovery_action() {
        let response = ApiError::from(accounts::Error::OAuth {
            stage: accounts::OAuthStage::Exchange,
            detail: "invalid_grant: provider rejected [REDACTED]".into(),
        })
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = axum::body::to_bytes(response.into_body(), 4_096)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["stage"], "exchange");
        assert_eq!(
            body["detail"],
            "invalid_grant: provider rejected [REDACTED]"
        );
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("step 6 (token exchange)")
        );
        assert!(body["error"].as_str().unwrap().contains("retry linking"));
        assert_eq!(body.as_object().unwrap().len(), 3);
    }
}

impl From<integrations::Error> for ApiError {
    fn from(error: integrations::Error) -> Self {
        match error {
            integrations::Error::NotFound => {
                Self::Message(StatusCode::NOT_FOUND, "integration not found")
            }
            integrations::Error::Source(error) => error.into(),
            other => unavailable(other),
        }
    }
}
async fn integration_list(
    AxumState(state): AxumState<State>,
) -> Result<Json<Vec<integrations::Integration>>, ApiError> {
    Ok(Json(integrations::list(&state.sources).await?))
}
async fn add_integration(
    AxumState(state): AxumState<State>,
    Path(slug): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = integrations::add(&state.sources, &slug).await?;
    state.sources.wake();
    Ok(Json(serde_json::json!({"id":id})))
}

async fn configured_oauth_apps(
    AxumState(state): AxumState<State>,
) -> Result<Json<Vec<String>>, ApiError> {
    Ok(Json(state.accounts.configured_apps().await?))
}
async fn save_oauth_app(
    AxumState(state): AxumState<State>,
    Path(provider): Path<String>,
    ApiJson(input): ApiJson<accounts::OAuthAppInput>,
) -> Result<StatusCode, ApiError> {
    state.accounts.save_oauth_app(&provider, input).await?;
    state.sources.wake();
    Ok(StatusCode::NO_CONTENT)
}
async fn remove_oauth_app(
    AxumState(state): AxumState<State>,
    Path(provider): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.accounts.remove_oauth_app(&provider).await?;
    state.sources.wake();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionQuery {
    #[serde(default)]
    q: String,
    before: Option<Uuid>,
    #[serde(default)]
    errors_only: bool,
}
async fn agent_session_list(
    AxumState(state): AxumState<State>,
    Query(query): Query<SessionQuery>,
) -> Result<Json<agent_sessions::Page>, ApiError> {
    Ok(Json(
        state
            .agent_sessions
            .list_filtered(&query.q, query.before, query.errors_only)
            .await?,
    ))
}
async fn agent_session_activity(
    AxumState(state): AxumState<State>,
    Query(query): Query<LogQuery>,
) -> Result<Json<agent_sessions::ActivityPage>, ApiError> {
    Ok(Json(state.agent_sessions.activity(query.before).await?))
}
async fn agent_session_detail(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    Query(query): Query<LogQuery>,
) -> Result<Json<agent_sessions::Detail>, ApiError> {
    Ok(Json(state.agent_sessions.detail(id, query.before).await?))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionTitle {
    title: String,
}
async fn rename_agent_session(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(input): ApiJson<SessionTitle>,
) -> Result<StatusCode, ApiError> {
    state.agent_sessions.rename(id, &input.title).await?;
    Ok(StatusCode::NO_CONTENT)
}
impl From<agent_sessions::Error> for ApiError {
    fn from(error: agent_sessions::Error) -> Self {
        match error {
            agent_sessions::Error::NotFound => {
                Self::Message(StatusCode::NOT_FOUND, "session not found")
            }
            agent_sessions::Error::Invalid => Self::Message(
                StatusCode::BAD_REQUEST,
                "use a title of 1–160 characters without control characters and a search of at most 1024 bytes",
            ),
            other => unavailable(other),
        }
    }
}

impl From<bulk::Error> for ApiError {
    fn from(error: bulk::Error) -> Self {
        match error {
            bulk::Error::Invalid => Self::Message(
                StatusCode::BAD_REQUEST,
                "choose either distinct IDs or a search query and its exact match count",
            ),
            bulk::Error::Changed => Self::Message(
                StatusCode::CONFLICT,
                "selection changed or includes unavailable items; reload and select again",
            ),
            bulk::Error::Scope => Self::Message(
                StatusCode::BAD_REQUEST,
                "switch this workspace to selected tools before changing tool inclusion",
            ),
            bulk::Error::Database(error) => unavailable(error),
        }
    }
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BulkRequest<T> {
    selection: bulk::Selection<T>,
    expected_count: i64,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BulkPolicyRequest {
    selection: bulk::Selection<String>,
    expected_count: i64,
    action: policies::BulkAction,
}
async fn bulk_policies(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(input): ApiJson<BulkPolicyRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let changed = state
        .sources
        .policies()
        .bulk(id, input.selection, input.expected_count, input.action)
        .await?;
    Ok(Json(serde_json::json!({"changed":changed})))
}
async fn bulk_tokens(
    AxumState(state): AxumState<State>,
    ApiJson(input): ApiJson<BulkRequest<Uuid>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let changed = state
        .auth
        .revoke_many(input.selection, input.expected_count)
        .await?;
    Ok(Json(serde_json::json!({"changed":changed})))
}
async fn bulk_sources(
    AxumState(state): AxumState<State>,
    ApiJson(input): ApiJson<BulkRequest<Uuid>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let changed = state
        .sources
        .sync_many(input.selection, input.expected_count)
        .await?;
    Ok(Json(serde_json::json!({"changed":changed})))
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BulkScriptRequest {
    selection: bulk::Selection<Uuid>,
    expected_count: i64,
    revisions: Vec<scripts::Revision>,
}
async fn bulk_scripts(
    AxumState(state): AxumState<State>,
    Path(id): Path<Uuid>,
    ApiJson(input): ApiJson<BulkScriptRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let changed = state
        .scripts
        .remove_many(id, input.selection, input.expected_count, &input.revisions)
        .await?;
    Ok(Json(serde_json::json!({"changed":changed})))
}

pub(crate) async fn browser_user(
    sessions: &Sessions,
    identity_enabled: bool,
    token: &str,
) -> anyhow::Result<Option<sessions::User>> {
    if identity_enabled {
        Ok(sessions.identity_user(token).await?)
    } else {
        Ok(sessions.user(token).await?)
    }
}
