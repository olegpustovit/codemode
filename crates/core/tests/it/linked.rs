use super::remote::Upstream;
use super::*;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use codemode_core::{
    components::{
        accounts::Accounts,
        sources::{Source, Transport},
    },
    infrastructure::sandbox::PendingCall,
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const ALPHA: &str = "provider_credential_alpha";
const BETA: &str = "provider_credential_beta";
#[derive(Clone, Default)]
struct Provider {
    requests: Arc<AtomicUsize>,
    rotated: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    agent_tokens: Arc<AtomicUsize>,
}
async fn authenticate_provider(
    State(provider): State<Provider>,
    request: Request,
    next: Next,
) -> Response {
    provider.requests.fetch_add(1, Ordering::SeqCst);
    let expected = if provider.rotated.load(Ordering::SeqCst) {
        BETA
    } else {
        ALPHA
    };
    let auth = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth.starts_with("Bearer cm_") {
        provider.agent_tokens.fetch_add(1, Ordering::SeqCst);
    }
    if auth != format!("Bearer {expected}") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 1024 * 1024).await.unwrap();
    if !bytes.is_empty() {
        let message: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        if message["method"] == "tools/call" {
            provider.calls.fetch_add(1, Ordering::SeqCst);
            if message["params"]["name"] == "fail" {
                return Json(serde_json::json!({"jsonrpc":"2.0","id":message["id"],"result":{"isError":true,"content":[{"type":"text","text":format!("provider failed with {expected}")}]}})).into_response();
            }
        }
    }
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}

async fn ready(sources: &Sources, id: Uuid) {
    let revision = sources.get(id).await.unwrap().unwrap().revision;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if sources
                .lookup("private.echo")
                .await
                .is_some_and(|tool| tool.revision == revision)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("linked source should synchronize");
}
fn call(id: u32, path: &str) -> PendingCall {
    PendingCall {
        id,
        path: path.into(),
        args: RawValue::from_string(
            if path.ends_with("echo") {
                r#"{"value":"linked"}"#
            } else {
                "{}"
            }
            .into(),
        )
        .unwrap(),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn linked_credentials_dispatch_rotate_redact_and_stop_after_unlink(pool: sqlx::PgPool) {
    let db = database(&pool).await;
    let auth = Auth::new(db.clone());
    let (token_id, agent) = auth.create("agent", "test.example").await.unwrap();
    let execution:Uuid=sqlx::query_scalar("INSERT INTO executions(token_id,code,status) VALUES($1,'linked credential fixture','running') RETURNING id").bind(token_id).fetch_one(&pool).await.unwrap();
    let provider = Provider::default();
    let service = StreamableHttpService::new(
        || Ok(Upstream),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true),
    );
    let app =
        Router::new()
            .nest_service("/mcp", service)
            .route_layer(middleware::from_fn_with_state(
                provider.clone(),
                authenticate_provider,
            ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let tasks = TaskTracker::new();
    let server_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let catalogue = Catalogue::new(db.clone());
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
    let sources = Sources::new(
        db.clone(),
        catalogue.clone(),
        accounts.clone(),
        codemode_core::components::workspaces::Workspaces::new(db.clone(), "test.example".into())
            .unwrap(),
    );
    let source = sources
        .save(
            None,
            Source {
                slug: "private".into(),
                requires_account: true,
                transport: Transport::McpHttp {
                    url: format!("http://{address}/mcp"),
                    bearer_env: None,
                },
            },
        )
        .await
        .unwrap();
    let workspace = auth
        .authenticate(&agent, "test.example")
        .await
        .unwrap()
        .unwrap()
        .workspace_id;
    let source_task = sources.clone();
    let source_stop = stop.clone();
    tasks.spawn(async move {
        source_task.run(source_stop).await;
    });
    let (_, unlinked) = sources
        .dispatch(
            execution,
            0,
            call(0, "private.echo"),
            std::time::Duration::from_secs(1),
            workspace,
        )
        .await;
    assert!(
        unlinked
            .unwrap_err()
            .to_string()
            .contains(&format!("/sources/{source}/accounts"))
    );
    sources.ready().await;
    assert_eq!(provider.requests.load(Ordering::SeqCst), 0);
    let unlinked_record = sources.get(source).await.unwrap().unwrap();
    assert_eq!(unlinked_record.status, "needs_link");
    assert!(unlinked_record.last_error.is_none());
    accounts.link_bearer(source, ALPHA.into()).await.unwrap();
    let stored: (Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT ciphertext,nonce FROM accounts WHERE source_id=$1")
            .bind(source)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored.1.len(), 24);
    assert!(
        !stored
            .0
            .windows(ALPHA.len())
            .any(|bytes| bytes == ALPHA.as_bytes())
    );
    sources.wake();
    ready(&sources, source).await;
    let entries = codemode_core::components::integrations::list(&sources)
        .await
        .unwrap();
    let entry = entries
        .iter()
        .find(|entry| entry.source_id == Some(source))
        .unwrap();
    assert!(entry.tool_names.iter().any(|name| name == "private.echo"));
    assert!(entry.entry.url.is_empty());
    let (_, echo) = sources
        .dispatch(
            execution,
            1,
            call(1, "private.echo"),
            std::time::Duration::from_secs(1),
            workspace,
        )
        .await;
    assert_eq!(echo.unwrap().get(), r#""linked""#);
    let (_, failure) = sources
        .dispatch(
            execution,
            2,
            call(2, "private.fail"),
            std::time::Duration::from_secs(1),
            workspace,
        )
        .await;
    let failure = failure.unwrap_err().to_string();
    assert!(failure.contains("[REDACTED]"));
    assert!(!failure.contains(ALPHA));
    accounts.link_bearer(source, BETA.into()).await.unwrap();
    provider.rotated.store(true, Ordering::SeqCst);
    sources.wake();
    ready(&sources, source).await;
    let (_, echo) = sources
        .dispatch(
            execution,
            3,
            call(3, "private.echo"),
            std::time::Duration::from_secs(1),
            workspace,
        )
        .await;
    assert_eq!(echo.unwrap().get(), r#""linked""#);
    let calls = provider.calls.load(Ordering::SeqCst);
    assert_eq!(calls, 3);
    accounts.unlink(source).await.unwrap();
    sources.wake();
    let (_, unlinked) = sources
        .dispatch(
            execution,
            4,
            call(4, "private.echo"),
            std::time::Duration::from_secs(1),
            workspace,
        )
        .await;
    assert!(unlinked.is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), calls);
    sources.remove(source).await.unwrap();
    assert!(
        catalogue
            .search("", false, None, None)
            .await
            .unwrap()
            .tools
            .is_empty()
    );
    let errors: Vec<Option<String>> =
        sqlx::query_scalar("SELECT error FROM tool_calls WHERE execution_id=$1")
            .bind(execution)
            .fetch_all(&pool)
            .await
            .unwrap();
    for error in errors.into_iter().flatten() {
        assert!(!error.contains(ALPHA) && !error.contains(BETA) && !error.contains(&agent));
    }
    assert_eq!(provider.agent_tokens.load(Ordering::SeqCst), 0);
    stop.cancel();
    tasks.close();
    tasks.wait().await;
}
