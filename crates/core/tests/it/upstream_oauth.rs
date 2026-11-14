use super::{remote::Upstream, *};
#[path = "../support/oauth_provider.rs"]
mod fixture;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use codemode_core::{
    components::{
        accounts::{Accounts, Error as AccountError, OAuthCompletion},
        sources::{Source, Transport},
    },
    infrastructure::sandbox::PendingCall,
};
use fixture::{CLIENT_SECRET, Provider, REPLACEMENT, SESSION};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::{sync::atomic::Ordering, time::Duration};

async fn authenticate(State(provider): State<Provider>, request: Request, next: Next) -> Response {
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if token.starts_with("Bearer cm_") || token.starts_with("Bearer cmo_") {
        provider.leaked_agents.fetch_add(1, Ordering::SeqCst);
    }
    let expected = if provider.replacement.load(Ordering::SeqCst) {
        REPLACEMENT.into()
    } else {
        provider.access()
    };
    if token != format!("Bearer {expected}") {
        return provider.challenge();
    }
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 1024 * 1024).await.unwrap();
    if !body.is_empty() {
        let message: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if message["method"] == "tools/call" {
            provider.calls.fetch_add(1, Ordering::SeqCst);
            if message["params"]["name"] == "fail" {
                return Json(serde_json::json!({"jsonrpc":"2.0","id":message["id"],"result":{"isError":true,"content":[{"type":"text","text":format!("failed with {expected}")}]}})).into_response();
            }
        }
    }
    next.run(Request::from_parts(parts, Body::from(body))).await
}
struct Context {
    workspace: Uuid,
    db: Database,
    accounts: Accounts,
    sources: Sources,
    provider: Provider,
    source: Uuid,
    stop: CancellationToken,
    tasks: TaskTracker,
}
async fn setup(pool: &sqlx::PgPool, cimd: bool) -> Context {
    let db = database(pool).await;
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
    let workspaces =
        codemode_core::components::workspaces::Workspaces::new(db.clone(), "test.example".into())
            .unwrap();
    workspaces.initialize().await.unwrap();
    let workspace = workspaces.default_id().await.unwrap();
    let catalogue = Catalogue::new(db.clone());
    let sources = Sources::new(
        db.clone(),
        catalogue,
        accounts.clone(),
        codemode_core::components::workspaces::Workspaces::new(db.clone(), "test.example".into())
            .unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = Provider::new(format!("http://{}", listener.local_addr().unwrap()), cimd);
    let service = StreamableHttpService::new(
        || Ok(Upstream),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true),
    );
    let app = provider
        .routes()
        .merge(Router::new().nest_service("/mcp", service).route_layer(
            middleware::from_fn_with_state(provider.clone(), authenticate),
        ));
    let tasks = TaskTracker::new();
    let stop = CancellationToken::new();
    let server_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let source = sources
        .save(
            None,
            Source {
                slug: "private".into(),
                requires_account: true,
                transport: Transport::McpHttp {
                    url: format!("{}/mcp", provider.origin),
                    bearer_env: None,
                },
            },
        )
        .await
        .unwrap();
    Context {
        workspace,
        db,
        accounts,
        sources,
        provider,
        source,
        stop,
        tasks,
    }
}
async fn begin(context: &Context) -> String {
    let authorization = context
        .accounts
        .start_oauth(context.source, "test.example", SESSION)
        .await
        .unwrap();
    context.provider.prepare(&authorization)
}
async fn complete(context: &Context, state: &str) {
    assert!(
        matches!(context.accounts.complete_oauth(state,SESSION,Some("fixture-code"),Some(&context.provider.origin),false).await.unwrap(),OAuthCompletion::Linked(id) if id==context.source)
    );
}
async fn ready(context: &Context) {
    let revision = context
        .sources
        .get(context.source)
        .await
        .unwrap()
        .unwrap()
        .revision;
    tokio::time::timeout(Duration::from_secs(2), async {
        while !context
            .sources
            .lookup("private.echo")
            .await
            .is_some_and(|tool| tool.revision == revision)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn call(id: u32, path: &str) -> PendingCall {
    PendingCall {
        id,
        path: path.into(),
        args: RawValue::from_string(if path.ends_with("echo") {
            "{\"value\":\"oauth\"}".into()
        } else {
            "{}".into()
        })
        .unwrap(),
    }
}
async fn execution(context: &Context, pool: &sqlx::PgPool) -> Uuid {
    let (token, _) = Auth::new(context.db.clone())
        .create("agent", "test.example")
        .await
        .unwrap();
    sqlx::query_scalar("INSERT INTO executions(token_id,code,status) VALUES($1,'upstream oauth fixture','running') RETURNING id").bind(token).fetch_one(pool).await.unwrap()
}
fn run_sources(context: &Context) {
    let sources = context.sources.clone();
    let stop = context.stop.clone();
    context.tasks.spawn(async move {
        sources.run(stop).await;
    });
}
async fn finish(context: Context) {
    context.stop.cancel();
    context.tasks.close();
    context.tasks.wait().await;
}
async fn expire(pool: &sqlx::PgPool, id: Uuid) {
    sqlx::query("UPDATE accounts SET expires_at=now()-interval '1 second' WHERE source_id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn upstream_oauth_dcr_encrypts_refreshes_once_redacts_and_reauthenticates_on_rejection(
    pool: sqlx::PgPool,
) {
    let context = setup(&pool, false).await;
    let state = begin(&context).await;
    complete(&context, &state).await;
    assert_eq!(context.provider.registrations.load(Ordering::SeqCst), 1);
    let metadata = context.accounts.list().await.unwrap();
    assert_eq!(metadata[0].kind, "oauth");
    assert!(metadata[0].expires_at.is_some());
    let stored: (Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT ciphertext,nonce FROM accounts WHERE source_id=$1")
            .bind(context.source)
            .fetch_one(&pool)
            .await
            .unwrap();
    for secret in ["provider-access-1", "provider-refresh-1", CLIENT_SECRET] {
        assert!(
            !stored
                .0
                .windows(secret.len())
                .any(|w| w == secret.as_bytes())
        );
    }
    run_sources(&context);
    ready(&context).await;
    let execution = execution(&context, &pool).await;
    expire(&pool, context.source).await;
    let calls = (0..8).map(|n| {
        context.sources.dispatch(
            execution,
            n,
            call(n as u32, "private.echo"),
            Duration::from_secs(2),
            context.workspace,
        )
    });
    for (_, result) in futures_util::future::join_all(calls).await {
        assert_eq!(result.unwrap().get(), "\"oauth\"");
    }
    assert_eq!(context.provider.refreshes.load(Ordering::SeqCst), 1);
    let changed: Vec<u8> = sqlx::query_scalar("SELECT nonce FROM accounts WHERE source_id=$1")
        .bind(context.source)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(stored.1, changed);
    let (_, failed) = context
        .sources
        .dispatch(
            execution,
            8,
            call(8, "private.fail"),
            Duration::from_secs(2),
            context.workspace,
        )
        .await;
    let failed = failed.unwrap_err();
    assert!(failed.contains("[REDACTED]"));
    assert!(!failed.contains("provider-access-2"));
    expire(&pool, context.source).await;
    context.provider.transient.store(true, Ordering::SeqCst);
    let (_, failed) = context
        .sources
        .dispatch(
            execution,
            9,
            call(9, "private.echo"),
            Duration::from_secs(2),
            context.workspace,
        )
        .await;
    assert!(
        failed
            .unwrap_err()
            .contains(&format!("/sources/{}/accounts", context.source))
    );
    assert_eq!(context.accounts.list().await.unwrap().len(), 1);
    context.provider.transient.store(false, Ordering::SeqCst);
    let (_, result) = context
        .sources
        .dispatch(
            execution,
            10,
            call(10, "private.echo"),
            Duration::from_secs(2),
            context.workspace,
        )
        .await;
    assert!(result.is_ok());
    expire(&pool, context.source).await;
    context.provider.rejected.store(true, Ordering::SeqCst);
    let (_, failed) = context
        .sources
        .dispatch(
            execution,
            11,
            call(11, "private.echo"),
            Duration::from_secs(2),
            context.workspace,
        )
        .await;
    assert!(failed.is_err());
    assert!(context.accounts.list().await.unwrap().is_empty());
    let errors: String = sqlx::query_scalar(
        "SELECT COALESCE(string_agg(error,' '),'') FROM tool_calls WHERE execution_id=$1",
    )
    .bind(execution)
    .fetch_one(&pool)
    .await
    .unwrap();
    for secret in [
        "provider-access-1",
        "provider-access-2",
        "provider-refresh-2",
        CLIENT_SECRET,
    ] {
        assert!(!errors.contains(secret));
    }
    assert_eq!(context.provider.leaked_agents.load(Ordering::SeqCst), 0);
    finish(context).await;
}

#[sqlx::test(migrations = "./migrations")]
async fn upstream_oauth_cimd_callback_state_is_session_bound_single_use_expiring_and_restart_safe(
    pool: sqlx::PgPool,
) {
    let context = setup(&pool, true).await;
    let state = begin(&context).await;
    let pending: (Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT state_hash,ciphertext FROM upstream_oauth_states WHERE source_id=$1",
    )
    .bind(context.source)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending.0, Sha256::digest(state.as_bytes()).to_vec());
    let mut tampered = pending.1.clone();
    tampered[0] ^= 1;
    sqlx::query("UPDATE upstream_oauth_states SET ciphertext=$1 WHERE source_id=$2")
        .bind(tampered)
        .bind(context.source)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        context
            .accounts
            .complete_oauth(
                &state,
                SESSION,
                Some("fixture-code"),
                Some(&context.provider.origin),
                false
            )
            .await,
        Err(AccountError::Authentication)
    ));
    sqlx::query("UPDATE upstream_oauth_states SET ciphertext=$1 WHERE source_id=$2")
        .bind(&pending.1)
        .bind(context.source)
        .execute(&pool)
        .await
        .unwrap();
    for secret in [&state, SESSION] {
        assert!(
            !pending
                .1
                .windows(secret.len())
                .any(|w| w == secret.as_bytes())
        );
    }
    assert!(matches!(
        context
            .accounts
            .complete_oauth(
                &state,
                "different-browser",
                Some("fixture-code"),
                Some(&context.provider.origin),
                false
            )
            .await,
        Err(AccountError::State)
    ));
    assert!(matches!(
        context
            .accounts
            .complete_oauth(
                &state,
                SESSION,
                Some("fixture-code"),
                Some("https://forged.example"),
                false
            )
            .await,
        Err(AccountError::State)
    ));
    assert!(matches!(
        context
            .accounts
            .complete_oauth(&state, SESSION, Some("fixture-code"), None, false)
            .await,
        Err(AccountError::State)
    ));
    let restarted = Accounts::new(context.db.clone(), Some([73; 32]), "test".into()).unwrap();
    assert!(matches!(
        restarted
            .complete_oauth(
                &state,
                SESSION,
                Some("fixture-code"),
                Some(&context.provider.origin),
                false
            )
            .await
            .unwrap(),
        OAuthCompletion::Linked(_)
    ));
    assert!(matches!(
        context
            .accounts
            .complete_oauth(
                &state,
                SESSION,
                Some("fixture-code"),
                Some(&context.provider.origin),
                false
            )
            .await,
        Err(AccountError::State)
    ));
    assert_eq!(context.provider.registrations.load(Ordering::SeqCst), 0);
    let state = begin(&context).await;
    assert!(matches!(
        context
            .accounts
            .complete_oauth(&state, SESSION, None, Some(&context.provider.origin), true)
            .await
            .unwrap(),
        OAuthCompletion::Denied(_)
    ));
    let state = begin(&context).await;
    sqlx::query("UPDATE upstream_oauth_states SET expires_at=now()-interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        context
            .accounts
            .complete_oauth(
                &state,
                SESSION,
                Some("fixture-code"),
                Some(&context.provider.origin),
                false
            )
            .await,
        Err(AccountError::State)
    ));
    context.accounts.sweep_oauth().await.unwrap();
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM upstream_oauth_states")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
    let state = begin(&context).await;
    let mut changed = context
        .sources
        .get(context.source)
        .await
        .unwrap()
        .unwrap()
        .source;
    changed.transport = Transport::McpHttp {
        url: format!("{}/other/mcp", context.provider.origin),
        bearer_env: None,
    };
    context
        .sources
        .save(Some(context.source), changed)
        .await
        .unwrap();
    assert!(matches!(
        context
            .accounts
            .complete_oauth(
                &state,
                SESSION,
                Some("fixture-code"),
                Some(&context.provider.origin),
                false
            )
            .await,
        Err(AccountError::Changed)
    ));
    assert_eq!(context.provider.exchanges.load(Ordering::SeqCst), 1);
    finish(context).await;
}

#[sqlx::test(migrations = "./migrations")]
async fn upstream_oauth_refresh_cannot_overwrite_a_replacement_account(pool: sqlx::PgPool) {
    let context = setup(&pool, false).await;
    let state = begin(&context).await;
    complete(&context, &state).await;
    run_sources(&context);
    ready(&context).await;
    let execution = execution(&context, &pool).await;
    expire(&pool, context.source).await;
    context.provider.blocked.store(true, Ordering::SeqCst);
    let sources = context.sources.clone();
    let workspace = context.workspace;
    let call = context.tasks.spawn(async move {
        sources
            .dispatch(
                execution,
                0,
                call(0, "private.echo"),
                Duration::from_secs(2),
                workspace,
            )
            .await
    });
    context.provider.refresh_entered.notified().await;
    context
        .accounts
        .link_bearer(context.source, REPLACEMENT.into())
        .await
        .unwrap();
    context.provider.replacement.store(true, Ordering::SeqCst);
    context.provider.blocked.store(false, Ordering::SeqCst);
    context.provider.release.add_permits(1);
    assert!(call.await.unwrap().1.is_err());
    assert_eq!(context.accounts.list().await.unwrap()[0].kind, "bearer");
    context.sources.wake();
    ready(&context).await;
    let (_, result) = context
        .sources
        .dispatch(
            execution,
            1,
            self::call(1, "private.echo"),
            Duration::from_secs(2),
            context.workspace,
        )
        .await;
    assert!(result.is_ok());
    finish(context).await;
}
