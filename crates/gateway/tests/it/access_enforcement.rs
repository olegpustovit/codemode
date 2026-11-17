use super::{identity, server};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use codemode_core::{
    components::{
        accounts::Accounts,
        auth::{Auth, oauth::OAuth, rate_limit::RateLimiter},
        catalogue::Catalogue,
        execution::Execution,
        sessions::Sessions,
        sources::Sources,
        workspaces::Workspaces,
    },
    infrastructure::{postgres::Database, sandbox::MontySandbox},
};
use idclient::{AppError, Application, Claims};
use secrecy::ExposeSecret;
use sqlx::ConnectOptions;
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;
#[path = "../../../core/tests/support/access_provider.rs"]
mod provider;

#[sqlx::test(migrations = "../core/migrations")]
async fn access_cli_denial_precedes_bootstrap_workspace_writes(pool: sqlx::PgPool) {
    let binary = std::env::var_os("CODEMODE_GATEWAY_BIN")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_codemode").into());
    let mut command = tokio::process::Command::new(&binary);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env(
            "CODEMODE_DATABASE_URL",
            pool.connect_options().to_url_lossy().as_str(),
        )
        .env("CODEMODE_ID_ENABLED", "true")
        .env("CODEMODE_ID_ACCESS_ENABLED", "true")
        .env("CODEMODE_ID_ISSUER", "http://127.0.0.1:1")
        .env("CODEMODE_ID_CLIENT_ID", "eval-client")
        .env("CODEMODE_ID_CLIENT_SECRET", "eval-fixture")
        .env("CODEMODE_ID_COOKIE_KEY", "31".repeat(32))
        .args(["token", "create", "eval-denied"]);
    let denied = command.output().await.unwrap();
    assert!(!denied.status.success());
    assert!(denied.stdout.is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM endpoints")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM access_tokens")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[sqlx::test(migrations = "../core/migrations")]
async fn access_callback_checks_before_provision_and_session_rotation_and_keeps_logout(
    pool: sqlx::PgPool,
) {
    let db = Database::connect(pool.connect_options().to_url_lossy().as_str())
        .await
        .unwrap();
    let provider = provider::Provider::new(db.clone()).await;
    let sessions = Sessions::new(db);
    let app = identity::App::new(sessions.clone()).with_access(provider.access.clone());
    let claims = || Claims {
        subject: "usr_eval".into(),
        email: "eval-callback@example.test".into(),
        email_verified: true,
        name: "eval".into(),
        picture: None,
        amr: vec![],
    };
    for mode in [1, 2] {
        provider.mode(mode);
        assert!(matches!(
            app.resolve_user(claims()).await,
            Err(AppError::Denied | AppError::Unavailable)
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }
    provider.mode(0);
    let subject = app.resolve_user(claims()).await.unwrap();
    let original = sessions
        .create_identity_session(&subject)
        .await
        .unwrap()
        .unwrap();
    let parts = || {
        Request::builder()
            .header(
                "cookie",
                format!("__Host-codemode_id_session={}", original.expose_secret()),
            )
            .body(())
            .unwrap()
            .into_parts()
            .0
    };
    for mode in [1, 2] {
        provider.mode(mode);
        assert!(matches!(
            app.create_session(subject.clone(), parts()).await,
            Err(AppError::Denied | AppError::Unavailable)
        ));
        assert!(
            sessions
                .identity_user(original.expose_secret())
                .await
                .unwrap()
                .is_some()
        );
    }
    let cleared = app.clear_session(parts()).await.unwrap();
    assert_eq!(cleared.len(), 1);
    assert!(
        cleared[0]
            .to_str()
            .unwrap()
            .starts_with("__Host-codemode_id_session=;")
    );
    assert!(
        sessions
            .identity_user(original.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    provider.close().await;
}

#[sqlx::test(migrations = "../core/migrations")]
async fn access_cookie_bearer_and_cached_tenant_router_recover_without_replacing_data(
    pool: sqlx::PgPool,
) {
    let db = Database::connect(pool.connect_options().to_url_lossy().as_str())
        .await
        .unwrap();
    provider::owner(&pool, uuid::Uuid::nil(), "usr_eval").await;
    let provider = provider::Provider::new(db.clone()).await;
    let (state, sandbox, cookie, token, saved) = setup_state(db, &provider).await;
    let sessions = state.sessions.clone();
    let mcp_tasks = state.mcp_tasks.clone();
    let router = server::router(state);
    let browser = |path: &str, method: &str| {
        Request::builder()
            .uri(path)
            .method(method)
            .header("host", "test.example")
            .header("origin", "https://test.example")
            .header(
                "cookie",
                format!("__Host-codemode_id_session={}", cookie.expose_secret()),
            )
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"eval-denied","sourceIds":[]}"#))
            .unwrap()
    };
    let bearer = || {
        Request::builder()
            .uri("/mcp")
            .method("POST")
            .header("host", "test.example")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25")
            .body(Body::from(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            ))
            .unwrap()
    };
    for (mode, expected) in [
        (0, StatusCode::OK),
        (1, StatusCode::FORBIDDEN),
        (2, StatusCode::SERVICE_UNAVAILABLE),
        (0, StatusCode::OK),
    ] {
        provider.mode(mode);
        for request in [
            browser("/api/session", "GET"),
            browser("/api/workspaces", "GET"),
            bearer(),
        ] {
            let response = router.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), expected);
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            if expected == StatusCode::OK && bytes.starts_with(b"[") {
                assert!(
                    String::from_utf8(bytes.to_vec())
                        .unwrap()
                        .contains(&saved.to_string())
                );
            }
        }
        if mode != 0 {
            let response = router
                .clone()
                .oneshot(browser("/api/workspaces", "POST"))
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
            to_bytes(response.into_body(), 65536).await.unwrap();
        }
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM endpoints WHERE slug='eval-denied'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert!(
        sessions
            .identity_user(cookie.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    provider.mode(1);
    let logout = router
        .clone()
        .oneshot(browser("/api/session", "DELETE"))
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert!(
        sessions
            .identity_user(cookie.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM endpoints WHERE id=$1")
            .bind(saved)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    mcp_tasks.shutdown();
    provider.close().await;
    sandbox.pool.close().await;
}

async fn setup_state(
    db: Database,
    provider: &provider::Provider,
) -> (
    server::State,
    Arc<MontySandbox>,
    secrecy::SecretString,
    String,
    uuid::Uuid,
) {
    let endpoints = Workspaces::new(db.clone(), "test.example".into()).unwrap();
    endpoints.initialize().await.unwrap();
    let saved = endpoints.create("eval-saved", &[]).await.unwrap();
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "eval".into())
        .unwrap()
        .with_access(provider.access.clone());
    let catalogue = Catalogue::new(db.clone());
    let sources = Sources::new(
        db.clone(),
        catalogue.clone(),
        accounts.clone(),
        endpoints.clone(),
    );
    let worker = std::env::var_os("CODEMODE_MONTY_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.tools/monty/bin/monty")
        });
    let sandbox = Arc::new(
        MontySandbox::new(&worker, Duration::from_secs(60), 32768)
            .await
            .unwrap(),
    );
    let execution = Arc::new(
        Execution::new(
            db.clone(),
            sources.clone(),
            sandbox.clone(),
            provider.tasks.clone(),
            provider.stop.clone(),
            Default::default(),
        )
        .with_access(provider.access.clone()),
    );
    let auth = Auth::new(db.clone()).with_access(provider.access.clone());
    let (_, token) = auth.create("eval-http", "test.example").await.unwrap();
    let sessions = Arc::new(Sessions::new(db.clone()));
    let cookie = sessions
        .create_identity_session("usr_eval")
        .await
        .unwrap()
        .unwrap();
    let state = server::State {
        access: provider.access.clone(),
        mcp_tasks: server::Tasks::default(),
        db: db.clone(),
        rate_limit: Arc::new(RateLimiter::new(600, 600, 6000)),
        auth,
        catalogue,
        execution,
        hostname: "test.example".into(),
        sessions: sessions.clone(),
        identity_enabled: true,
        identity_routes: None,
        accounts: Arc::new(accounts),
        sources: Arc::new(sources),
        workspaces: Arc::new(endpoints),
        stop: provider.stop.clone(),
        oauth: Arc::new(OAuth::new(db, "test.example".into()).with_access(provider.access.clone())),
    };
    (state, sandbox, cookie, token, saved)
}

#[sqlx::test(migrations = "../core/migrations")]
#[ignore = "real 25-second revocation with HTTP readers and SDK tasks"]
async fn access_http_active_requests_and_detached_task_removal(pool: sqlx::PgPool) {
    live_http(pool, 1).await;
}
#[sqlx::test(migrations = "../core/migrations")]
#[ignore = "real 25-second outage with HTTP readers and SDK tasks"]
async fn access_http_active_requests_and_detached_task_outage(pool: sqlx::PgPool) {
    live_http(pool, 2).await;
}

async fn live_http(pool: sqlx::PgPool, mode: u8) {
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Instant,
    };
    use tokio_util::sync::CancellationToken;
    let db = Database::connect(pool.connect_options().to_url_lossy().as_str())
        .await
        .unwrap();
    provider::owner(&pool, uuid::Uuid::nil(), "usr_eval").await;
    let provider = provider::Provider::new(db.clone()).await;
    let (state, sandbox, cookie, token, saved) = setup_state(db, &provider).await;
    let principal = state
        .auth
        .authenticate(&token, "test.example")
        .await
        .unwrap()
        .unwrap();
    let mcp_tasks = state.mcp_tasks.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let stop = provider.stop.clone();
    provider.tasks.spawn(async move {
        axum::serve(listener, server::router(state))
            .with_graceful_shutdown(stop.cancelled_owned())
            .await
            .unwrap();
    });
    let client = reqwest::Client::new();
    let browser_cookie = format!("__Host-codemode_id_session={}", cookie.expose_secret());
    let mut capable =
        serde_json::json!({"name":"execute","arguments":{"code":"while True:\n    pass"}});
    capable["_meta"] = serde_json::json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{"extensions":{"io.modelcontextprotocol/tasks":{}}},"io.modelcontextprotocol/clientInfo":{"name":"eval-access","version":"1"}});
    let task = client
        .post(format!("{url}/mcp"))
        .header("host", "test.example")
        .bearer_auth(&token)
        .header("accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/call")
        .header("Mcp-Name", "execute")
        .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":capable}))
        .send()
        .await
        .unwrap();
    assert_eq!(task.status(), 200);
    let task: serde_json::Value = task.json().await.unwrap();
    let task_id = task["result"]["taskId"].as_str().unwrap().to_owned();
    let pending_client = client.clone();
    let pending_url = url.clone();
    let pending_token = token.clone();
    let pending = tokio::spawn(async move {
        pending_client.post(format!("{pending_url}/mcp")).header("host","test.example").bearer_auth(pending_token).header("accept","application/json, text/event-stream").header("MCP-Protocol-Version","2025-11-25").json(&serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"execute","arguments":{"code":"while True:\n    pass"}}})).send().await.unwrap()
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while sqlx::query_scalar::<_, i64>("SELECT count(*) FROM executions WHERE status='running'")
            .fetch_one(&pool)
            .await
            .unwrap()
            != 2
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let counts = Arc::new([
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    ]);
    let load_stop = CancellationToken::new();
    let mut readers = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let client = client.clone();
        let url = url.clone();
        let cookie = browser_cookie.clone();
        let stop = load_stop.clone();
        let counts = counts.clone();
        readers.spawn(async move {
            while !stop.is_cancelled() {
                let response=client.get(format!("{url}/api/workspaces")).header("host","test.example").header("cookie",&cookie).send().await.unwrap();
                let i=match response.status().as_u16() {200=>0,403=>1,503=>2,status=>panic!("unexpected reader status {status}")};
                counts[i].fetch_add(1,Ordering::SeqCst);
                response.bytes().await.unwrap();
                tokio::select! {biased;_=stop.cancelled()=>break,_=tokio::time::sleep(Duration::from_millis(50))=>{}}
            }
        });
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while counts[0].load(Ordering::SeqCst) < 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let start = Instant::now();
    provider.mode(mode);
    let stopped = tokio::time::timeout(Duration::from_secs(29), pending)
        .await
        .unwrap()
        .unwrap();
    let stopped_ms = start.elapsed().as_millis();
    assert_eq!(stopped.status().as_u16(), if mode == 1 { 403 } else { 503 });
    stopped.bytes().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let running = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM executions WHERE status='running'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            let task = mcp_tasks.get(&principal, &task_id).unwrap();
            let terminal = matches!(
                task.task.status,
                rmcp::model::TaskStatus::Failed | rmcp::model::TaskStatus::Cancelled
            );
            if running == 0 && terminal {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let task_stopped_ms = start.elapsed().as_millis();
    let poll=client.post(format!("{url}/mcp")).header("host","test.example").bearer_auth(&token).header("accept","application/json, text/event-stream").header("MCP-Protocol-Version","2026-07-28").header("Mcp-Method","tasks/get").header("Mcp-Name",&task_id).json(&serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tasks/get","params":{"taskId":task_id,"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{"extensions":{"io.modelcontextprotocol/tasks":{}}}}}})).send().await.unwrap();
    assert_eq!(poll.status().as_u16(), if mode == 1 { 403 } else { 503 });
    poll.bytes().await.unwrap();
    load_stop.cancel();
    while let Some(result) = readers.join_next().await {
        result.unwrap();
    }
    assert!(task_stopped_ms < 29_000);
    assert!(counts[if mode == 1 { 1 } else { 2 }].load(Ordering::SeqCst) > 100);
    provider.mode(0);
    let restored = client
        .get(format!("{url}/api/workspaces/{saved}"))
        .header("host", "test.example")
        .header("cookie", &browser_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(restored.status(), 200);
    assert_eq!(
        restored.json::<serde_json::Value>().await.unwrap()["id"],
        saved.to_string()
    );
    let report = serde_json::json!({"scope":"Actual Axum HTTP, official MCP detached task and Monty workers; owned policy transport fixture and PostgreSQL 18","mode":mode,"pendingHttpStopMs":stopped_ms,"taskAndActualWorkStopMs":task_stopped_ms,"httpReaders":4,"responses":{"allowed":counts[0].load(Ordering::SeqCst),"denied":counts[1].load(Ordering::SeqCst),"unavailable":counts[2].load(Ordering::SeqCst)},"runningExecutionsRemaining":0,"retainedSdkTaskTerminal":true,"sameCookieAndWorkspaceRecovered":true});
    if let Some(dir) = std::env::var_os("CODEMODE_ACCESS_HTTP_REPORT_DIR") {
        std::fs::write(
            std::path::PathBuf::from(dir).join(format!("access-http-{mode}-20261117.json")),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
    println!("access-http: {report}");
    mcp_tasks.shutdown();
    provider.close().await;
    sandbox.pool.close().await;
}
