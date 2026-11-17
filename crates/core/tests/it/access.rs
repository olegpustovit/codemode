use super::*;
use codemode_core::components::{access::Error as AccessError, auth::Error as AuthError};
#[path = "../support/access_provider.rs"]
mod provider;
#[path = "../support/access_upstream.rs"]
mod upstream;

async fn until(mut predicate: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !predicate().await {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "real 25-second watch retains the original subject across binding changes"]
async fn access_running_work_cannot_switch_to_another_allowed_subject(pool: sqlx::PgPool) {
    use std::time::{Duration, Instant};
    let db = database(&pool).await;
    provider::owner(&pool, Uuid::nil(), "usr_eval").await;
    let provider = provider::Provider::new(db).await;
    let mut watch = provider.access.watch(Uuid::nil()).await.unwrap();
    sqlx::query("UPDATE user_identities SET subject='usr_other' WHERE subject='usr_eval'")
        .execute(&pool)
        .await
        .unwrap();
    // New work may use the newly bound eligible identity, while already-running
    // work cannot silently move to that identity's grant.
    provider.access.tenant(Uuid::nil(), true).await.unwrap();
    let start = Instant::now();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(28), watch.stopped())
            .await
            .unwrap(),
        AccessError::Denied
    );
    assert!(start.elapsed() < Duration::from_secs(28));
    watch.close().await;
    provider.close().await;
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "real 25-second removal and outage leases with actual Monty and MCP transports"]
async fn access_live_workers_upstream_streams_and_source_removal(pool: sqlx::PgPool) {
    live_work(pool, 1).await;
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "real 25-second outage lease with actual Monty and MCP transports"]
async fn access_live_workers_upstream_streams_and_source_outage(pool: sqlx::PgPool) {
    live_work(pool, 2).await;
}

async fn live_work(pool: sqlx::PgPool, mode: u8) {
    use codemode_core::{
        components::{
            accounts::Accounts,
            sources::{Source, Transport},
            workspaces::Workspaces,
        },
        infrastructure::sandbox::MontySandbox,
    };
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };
    let db = database(&pool).await;
    provider::owner(&pool, Uuid::nil(), "usr_eval").await;
    let provider = provider::Provider::new(db.clone()).await;
    let upstream = upstream::Upstream::new().await;
    let endpoints = Workspaces::new(db.clone(), "test.example".into()).unwrap();
    endpoints.initialize().await.unwrap();
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "eval".into())
        .unwrap()
        .with_access(provider.access.clone());
    let catalogue = Catalogue::new(db.clone());
    let sources = Sources::new(db.clone(), catalogue.clone(), accounts.clone(), endpoints)
        .with_access(provider.access.clone(), Uuid::nil());
    let id = sources
        .save(
            None,
            Source {
                slug: "eval_tools".into(),
                requires_account: true,
                transport: Transport::McpHttp {
                    url: upstream.url.clone(),
                    bearer_env: None,
                },
            },
        )
        .await
        .unwrap();
    accounts
        .link_bearer(id, "eval-provider-token".into())
        .await
        .unwrap();
    let worker =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.tools/monty/bin/monty");
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
    execution.start_sources();
    tokio::time::timeout(Duration::from_secs(3), sources.ready())
        .await
        .unwrap();
    assert!(sources.lookup("eval_tools.wait").await.is_some());
    let auth = Auth::new(db.clone()).with_access(provider.access.clone());
    let (_, token) = auth.create("eval-worker", "test.example").await.unwrap();
    let principal = auth
        .authenticate(&token, "test.example")
        .await
        .unwrap()
        .unwrap();
    let mut measurements = Vec::new();
    for mode in [mode] {
        // Refresh initial checks and synchronize a real pending upstream body.
        let cpu = execution.clone();
        let actor = principal.clone();
        let cpu_task = tokio::spawn(async move {
            cpu.execute(actor, "while True:\n    pass".into())
                .await
                .unwrap()
        });
        let remote = execution.clone();
        let actor = principal.clone();
        let calls_before = upstream.counters.calls.load(Ordering::SeqCst);
        let remote_task = tokio::spawn(async move {
            remote
                .execute(actor, "return await tools.eval_tools.wait()".into())
                .await
                .unwrap()
        });
        until(async || upstream.counters.calls.load(Ordering::SeqCst) > calls_before).await;
        let start = Instant::now();
        provider.mode(mode);
        let (cpu, remote) = tokio::time::timeout(Duration::from_secs(29), async {
            (cpu_task.await.unwrap(), remote_task.await.unwrap())
        })
        .await
        .unwrap();
        let stopped_ms = start.elapsed().as_millis();
        assert!(stopped_ms < 29_000);
        assert_eq!(cpu.status, "failed");
        assert_eq!(remote.status, "failed");
        assert!(cpu.error.as_deref().unwrap().contains("cancelled"));
        until(async || {
            sources.lookup("eval_tools.wait").await.is_none()
                && upstream.counters.streams.load(Ordering::SeqCst) == 0
        })
        .await;
        assert!(upstream.counters.deleted.load(Ordering::SeqCst) > 0);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM executions WHERE status='running'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tool_calls WHERE outcome='running'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        // Resume the existing durable source and PAT, then prove the execution
        // permits and worker pool were returned rather than merely hiding output.
        provider.mode(0);
        sources.wake();
        tokio::time::timeout(Duration::from_secs(6), sources.ready())
            .await
            .unwrap();
        assert!(sources.lookup("eval_tools.wait").await.is_some());
        let restored = execution
            .execute(principal.clone(), "return 42".into())
            .await
            .unwrap();
        assert_eq!(restored.status, "completed");
        assert_eq!(restored.result.get(), "42");
        assert!(
            auth.authenticate(&token, "test.example")
                .await
                .unwrap()
                .is_some()
        );
        measurements.push(serde_json::json!({"mode":mode,"stoppedMs":stopped_ms,"actualWorker":true,"actualUpstreamSse":true,"streamsAtStop":0,"streamsAfterRecovery":upstream.counters.streams.load(Ordering::SeqCst)}));
    }
    provider.close().await;
    sandbox.pool.close().await;
    println!(
        "access-live: {}",
        serde_json::json!({"phases":measurements,"providerCalls":upstream.counters.calls.load(Ordering::SeqCst),"closedBodies":upstream.counters.closed.load(Ordering::SeqCst)})
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn access_owner_binding_and_admin_never_bypass_tenant_isolation(pool: sqlx::PgPool) {
    let db = database(&pool).await;
    let provider = provider::Provider::new(db.clone()).await;
    let access = &provider.access;
    assert_eq!(
        access.tenant(Uuid::nil(), true).await,
        Err(AccessError::Denied)
    );
    assert_eq!(
        provider
            .policy
            .checks
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    let user = provider::owner(&pool, Uuid::nil(), "usr_eval").await;
    let other = Uuid::new_v4();
    provider::owner(&pool, other, "usr_other").await;
    access.tenant(Uuid::nil(), true).await.unwrap();
    let scoped = db.for_tenant(Uuid::nil()).await.unwrap();
    let auth = Auth::new(scoped).with_access(access.clone());
    let (_, token) = auth.create("eval-pat", "test.example").await.unwrap();
    let stored_before: (i64, i64) =
        sqlx::query_as("SELECT (SELECT count(*) FROM users),(SELECT count(*) FROM access_tokens)")
            .fetch_one(&pool)
            .await
            .unwrap();
    provider.mode(1);
    assert_eq!(
        access.tenant(Uuid::nil(), true).await,
        Err(AccessError::Denied)
    );
    assert!(matches!(
        auth.authenticate(&token, "test.example").await,
        Err(AuthError::Access(AccessError::Denied))
    ));
    assert!(matches!(
        auth.create("eval-denied", "test.example").await,
        Err(AuthError::Access(AccessError::Denied))
    ));
    access.tenant(other, true).await.unwrap();
    provider.mode(2);
    assert_eq!(
        access.tenant(Uuid::nil(), true).await,
        Err(AccessError::Unavailable)
    );
    assert!(matches!(
        auth.create("eval-outage", "test.example").await,
        Err(AuthError::Access(AccessError::Unavailable))
    ));
    provider.mode(0);
    assert!(
        auth.authenticate(&token, "test.example")
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query("DELETE FROM user_identities WHERE user_id=$1")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        access.tenant(Uuid::nil(), true).await,
        Err(AccessError::Denied)
    );
    sqlx::query("INSERT INTO user_identities(subject,user_id) VALUES('usr_eval',$1)")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        auth.authenticate(&token, "test.example")
            .await
            .unwrap()
            .is_some()
    );
    let after: (i64, i64) =
        sqlx::query_as("SELECT (SELECT count(*) FROM users),(SELECT count(*) FROM access_tokens)")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored_before, after);
    // The original role cannot read the binding even though its composed access
    // service can consult the private pool; no grants were added by this change.
    assert!(
        !sqlx::query_scalar::<_, bool>(
            "SELECT has_table_privilege('codemode_tenant','user_identities','SELECT')"
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    );
    provider.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn access_code_and_refresh_denials_leave_valid_credentials_redeemable(pool: sqlx::PgPool) {
    use codemode_core::components::auth::oauth::{
        AuthMethod, AuthorizationRequest, ClientAuthentication, ClientMetadata, Error, OAuth,
        TokenRequest,
    };
    let db = database(&pool).await;
    provider::owner(&pool, Uuid::nil(), "usr_eval").await;
    let provider = provider::Provider::new(db.clone()).await;
    let workspaces =
        codemode_core::components::workspaces::Workspaces::new(db.clone(), "test.example".into())
            .unwrap();
    workspaces.initialize().await.unwrap();
    let oauth = OAuth::new(db.clone(), "test.example".into()).with_access(provider.access.clone());
    let registered = oauth
        .register(ClientMetadata {
            client_name: "eval-client".into(),
            redirect_uris: vec!["https://client.example/callback".into()],
            token_endpoint_auth_method: AuthMethod::None,
            grant_types: vec!["authorization_code".into(), "refresh_token".into()],
            response_types: vec!["code".into()],
        })
        .await
        .unwrap();
    let client = ClientAuthentication {
        client_id: registered.client_id.clone(),
        method: AuthMethod::None,
        secret: None,
    };
    let request = AuthorizationRequest {
        workspace_id: Some(workspaces.default_id().await.unwrap()),
        response_type: "code".into(),
        client_id: registered.client_id,
        redirect_uri: "https://client.example/callback".into(),
        resource: "https://test.example/mcp".into(),
        code_challenge: String::new(),
        code_challenge_method: "S256".into(),
        state: None,
        scope: Some("mcp".into()),
    };
    // RFC 7636 vector, preserving the existing PKCE boundary.
    let mut request = request;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    request.code_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier));
    let code = oauth.issue_code(&request).await.unwrap();
    let exchange = TokenRequest {
        grant_type: "authorization_code".into(),
        resource: request.resource.clone(),
        code: Some(code.into()),
        redirect_uri: Some(request.redirect_uri),
        code_verifier: Some(verifier.into()),
        refresh_token: None,
        scope: None,
    };
    for (mode, expected) in [(1, AccessError::Denied), (2, AccessError::Unavailable)] {
        provider.mode(mode);
        assert!(
            matches!(oauth.token(&client, &exchange).await, Err(Error::Access(error)) if error==expected)
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM oauth_codes WHERE consumed_at IS NOT NULL"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    provider.mode(0);
    let original = oauth.token(&client, &exchange).await.unwrap();
    let refresh = TokenRequest {
        grant_type: "refresh_token".into(),
        resource: request.resource,
        code: None,
        redirect_uri: None,
        code_verifier: None,
        refresh_token: original.refresh_token.map(Into::into),
        scope: None,
    };
    for (mode, expected) in [(1, AccessError::Denied), (2, AccessError::Unavailable)] {
        provider.mode(mode);
        assert!(
            matches!(oauth.token(&client, &refresh).await, Err(Error::Access(error)) if error==expected)
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM oauth_tokens WHERE consumed_at IS NOT NULL"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    provider.mode(0);
    let rotated = oauth.token(&client, &refresh).await.unwrap();
    assert_ne!(original.access_token, rotated.access_token);
    assert!(matches!(
        oauth.token(&client, &refresh).await,
        Err(Error::Grant)
    ));
    assert!(
        Auth::new(db)
            .authenticate(&rotated.access_token, "test.example")
            .await
            .unwrap()
            .is_none()
    );
    provider.close().await;
}
