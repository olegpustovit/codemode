use axum::{
    Form, Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::{Notify, Semaphore};

pub const SESSION: &str = "fixture-browser-session";
pub const CLIENT_SECRET: &str = "fixture-client-secret";
pub const REPLACEMENT: &str = "replacement-bearer";
const CALLBACK: &str = "https://test.example/api/accounts/oauth/callback";
const CLIENT_METADATA: &str = "https://test.example/oauth/client-metadata.json";

#[derive(Clone)]
pub struct Provider {
    pub origin: String,
    pub cimd: bool,
    pub static_app: Arc<AtomicBool>,
    pub registrations: Arc<AtomicUsize>,
    pub exchanges: Arc<AtomicUsize>,
    pub refreshes: Arc<AtomicUsize>,
    pub generation: Arc<AtomicUsize>,
    pub calls: Arc<AtomicUsize>,
    pub leaked_agents: Arc<AtomicUsize>,
    pub transient: Arc<AtomicBool>,
    pub fail_registration: Arc<AtomicBool>,
    pub fail_exchange: Arc<AtomicBool>,
    pub rejected: Arc<AtomicBool>,
    pub replacement: Arc<AtomicBool>,
    pub blocked: Arc<AtomicBool>,
    pub refresh_entered: Arc<Notify>,
    pub release: Arc<Semaphore>,
    challenge: Arc<Mutex<String>>,
}
impl Provider {
    pub fn new(origin: String, cimd: bool) -> Self {
        Self {
            origin,
            cimd,
            static_app: Arc::new(AtomicBool::new(false)),
            registrations: Arc::new(AtomicUsize::new(0)),
            exchanges: Arc::new(AtomicUsize::new(0)),
            refreshes: Arc::new(AtomicUsize::new(0)),
            generation: Arc::new(AtomicUsize::new(0)),
            calls: Arc::new(AtomicUsize::new(0)),
            leaked_agents: Arc::new(AtomicUsize::new(0)),
            transient: Arc::new(AtomicBool::new(false)),
            fail_registration: Arc::new(AtomicBool::new(false)),
            fail_exchange: Arc::new(AtomicBool::new(false)),
            rejected: Arc::new(AtomicBool::new(false)),
            replacement: Arc::new(AtomicBool::new(false)),
            blocked: Arc::new(AtomicBool::new(false)),
            refresh_entered: Arc::new(Notify::new()),
            release: Arc::new(Semaphore::new(0)),
            challenge: Arc::new(Mutex::new(String::new())),
        }
    }
    pub fn routes(&self) -> Router {
        Router::new()
            .route("/.well-known/oauth-protected-resource", get(resource))
            .route("/.well-known/oauth-protected-resource/mcp", get(resource))
            .route("/.well-known/oauth-authorization-server", get(metadata))
            .route("/register", post(register))
            .route("/token", post(token))
            .with_state(self.clone())
    }
    pub fn prepare(&self, url: &str) -> String {
        let parsed = url::Url::parse(url).unwrap();
        let params: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(params["redirect_uri"], CALLBACK);
        assert_eq!(params["resource"], format!("{}/mcp", self.origin));
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(
            params["client_id"],
            if self.cimd {
                CLIENT_METADATA
            } else {
                "fixture-client"
            }
        );
        assert!(params["scope"].split(' ').any(|s| s == "tools"));
        assert!(params["scope"].split(' ').any(|s| s == "offline_access"));
        *self.challenge.lock().unwrap() = params["code_challenge"].clone();
        params["state"].clone()
    }
    pub fn challenge(&self) -> Response {
        (StatusCode::UNAUTHORIZED,[("www-authenticate",format!("Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\", scope=\"tools\"",self.origin))]).into_response()
    }
    pub fn access(&self) -> String {
        format!("provider-access-{}", self.generation.load(Ordering::SeqCst))
    }
}
async fn resource(State(provider): State<Provider>) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"resource":format!("{}/mcp",provider.origin),"authorization_servers":[provider.origin],"scopes_supported":["tools"]}),
    )
}
async fn metadata(State(provider): State<Provider>) -> Json<serde_json::Value> {
    let origin = &provider.origin;
    Json(
        serde_json::json!({"issuer":origin,"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token"),"registration_endpoint":format!("{origin}/register"),"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"code_challenge_methods_supported":["S256"],"scopes_supported":["tools","offline_access"],"token_endpoint_auth_methods_supported":["none","client_secret_basic"],"authorization_response_iss_parameter_supported":true,"client_id_metadata_document_supported":provider.cimd}),
    )
}
async fn register(
    State(provider): State<Provider>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if provider.fail_registration.load(Ordering::SeqCst) {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid_client_metadata", "error_description":"redirect URI is not accepted", "client_secret":"registration-secret"}))).into_response();
    }
    provider.registrations.fetch_add(1, Ordering::SeqCst);
    assert_eq!(body["application_type"], "web");
    assert_eq!(body["redirect_uris"], serde_json::json!([CALLBACK]));
    Json(
        serde_json::json!({"client_id":"fixture-client","client_secret":CLIENT_SECRET,"redirect_uris":[CALLBACK],"token_endpoint_auth_method":"client_secret_basic"}),
    ).into_response()
}
async fn token(
    State(provider): State<Provider>,
    headers: HeaderMap,
    Form(body): Form<HashMap<String, String>>,
) -> Response {
    if provider.static_app.load(Ordering::SeqCst) {
        assert_eq!(
            body.get("client_id").map(String::as_str),
            Some("fixture-client")
        );
        assert_eq!(
            body.get("client_secret").map(String::as_str),
            Some(CLIENT_SECRET)
        );
        assert!(!headers.contains_key("authorization"));
    } else if provider.cimd {
        assert_eq!(
            body.get("client_id").map(String::as_str),
            Some(CLIENT_METADATA)
        );
        assert!(!headers.contains_key("authorization"));
    } else {
        assert_eq!(
            headers["authorization"],
            format!(
                "Basic {}",
                STANDARD.encode(format!("fixture-client:{CLIENT_SECRET}"))
            )
        );
    }
    assert_eq!(body["resource"], format!("{}/mcp", provider.origin));
    if body["grant_type"] == "authorization_code" {
        assert_eq!(body["code"], "fixture-code");
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(body["code_verifier"].as_bytes())),
            *provider.challenge.lock().unwrap()
        );
        if provider.fail_exchange.load(Ordering::SeqCst) {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid_grant", "error_description":format!("denied code {} for secret {CLIENT_SECRET}", body["code"])}))).into_response();
        }
        provider.exchanges.fetch_add(1, Ordering::SeqCst);
    } else {
        assert_eq!(body["grant_type"], "refresh_token");
        provider.refreshes.fetch_add(1, Ordering::SeqCst);
        if provider.transient.load(Ordering::SeqCst) {
            return (StatusCode::SERVICE_UNAVAILABLE,Json(serde_json::json!({"error":"temporarily_unavailable","error_description":format!("secret {}",body["refresh_token"])}))).into_response();
        }
        if provider.rejected.load(Ordering::SeqCst) {
            return (StatusCode::BAD_REQUEST,Json(serde_json::json!({"error":"invalid_grant","error_description":format!("secret {}",body["refresh_token"])}))).into_response();
        }
        if body["refresh_token"]
            != format!(
                "provider-refresh-{}",
                provider.generation.load(Ordering::SeqCst)
            )
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":"invalid_grant"})),
            )
                .into_response();
        }
        if provider.blocked.load(Ordering::SeqCst) {
            provider.refresh_entered.notify_one();
            provider.release.acquire().await.unwrap().forget();
        }
    }
    let generation = provider.generation.fetch_add(1, Ordering::SeqCst) + 1;
    Json(serde_json::json!({"access_token":format!("provider-access-{generation}"),"refresh_token":format!("provider-refresh-{generation}"),"expires_in":3600,"token_type":"Bearer","scope":"tools offline_access"})).into_response()
}
