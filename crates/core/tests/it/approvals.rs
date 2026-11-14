use super::*;
use axum::{
    Json, Router,
    routing::{get, post},
};
use codemode_core::{
    components::{
        auth::Principal,
        execution::approvals::{Decision, Error as ApprovalError},
        policies::{Action, Choice},
        sources::{Source, Transport},
        workspaces::Workspaces,
    },
    infrastructure::sandbox::MontySandbox,
};
use serde_json::{Value, json};
use std::{sync::Mutex, time::Duration};

struct Fixture {
    db: Database,
    sources: Sources,
    catalogue: Catalogue,
    sandbox: Arc<MontySandbox>,
    execution: Arc<Execution<MontySandbox>>,
    principal: Principal,
    auth: Auth,
    endpoint: Uuid,
    source: Uuid,
    hits: Arc<Mutex<Vec<&'static str>>>,
    spec: Arc<Mutex<Value>>,
    tasks: TaskTracker,
    stop: CancellationToken,
}
pub(super) fn binary() -> std::path::PathBuf {
    std::env::var_os("CODEMODE_MONTY_BIN")
        .map(Into::into)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.tools/monty/bin/monty")
        })
}
impl Fixture {
    async fn new(pool: &sqlx::PgPool) -> Self {
        let db = database(pool).await;
        let endpoints = Workspaces::new(db.clone(), "test.example".into()).unwrap();
        endpoints.initialize().await.unwrap();
        let endpoint = endpoints.default_id().await.unwrap();
        let catalogue = Catalogue::new(db.clone());
        let sources = managed_sources(db.clone(), catalogue.clone());
        let tasks = TaskTracker::new();
        let stop = CancellationToken::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let spec = Arc::new(Mutex::new(
            json!({"openapi":"3.1.0","servers":[{"url":base}],"paths":{
                "/before":{"get":{"operationId":"before","responses":{}}},
                "/read":{"get":{"operationId":"read","responses":{}}},
                "/write":{"post":{"operationId":"write","x-destructive":true,"responses":{}}}
            }}),
        ));
        let document = spec.clone();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let mut router = Router::new().route(
            "/spec",
            get(move || {
                let document = document.clone();
                async move { Json(document.lock().unwrap().clone()) }
            }),
        );
        for (path, name) in [
            ("/before", "before"),
            ("/read", "read"),
            ("/write", "write"),
            ("/other", "other"),
        ] {
            let hits = hits.clone();
            let handler = move || {
                let hits = hits.clone();
                async move {
                    hits.lock().unwrap().push(name);
                    Json(json!(name))
                }
            };
            router = router.route(
                path,
                if name == "write" || name == "other" {
                    post(handler)
                } else {
                    get(handler)
                },
            );
        }
        let server_stop = stop.clone();
        tasks.spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(server_stop.cancelled_owned())
                .await
                .unwrap()
        });
        let source = sources
            .save(
                None,
                Source {
                    slug: "fixture".into(),
                    requires_account: false,
                    transport: Transport::OpenApi {
                        url: format!("{base}/spec"),
                        server_url: None,
                    },
                },
            )
            .await
            .unwrap();
        let sync = sources.clone();
        let sync_stop = stop.clone();
        tasks.spawn(async move { sync.run(sync_stop).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while sources.lookup("fixture.write").await.is_none() {
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap();
        let sandbox = Arc::new(
            MontySandbox::new(&binary(), Duration::from_secs(2), [73; 32], 32768)
                .await
                .unwrap(),
        );
        let execution = Arc::new(Execution::new(
            db.clone(),
            sources.clone(),
            sandbox.clone(),
            tasks.clone(),
            stop.clone(),
            Default::default(),
            [73; 32],
        ));
        let auth = Auth::new(db.clone());
        let (_, token) = auth.create("owner", "test.example").await.unwrap();
        let principal = auth
            .authenticate(&token, "test.example")
            .await
            .unwrap()
            .unwrap();
        Self {
            db,
            sources,
            catalogue,
            sandbox,
            execution,
            principal,
            auth,
            endpoint,
            source,
            hits,
            spec,
            tasks,
            stop,
        }
    }
    async fn pause(&self, code: &str) -> Uuid {
        let result = self
            .execution
            .execute(self.principal.clone(), code.into())
            .await
            .unwrap();
        assert_eq!(result.status, "paused", "{:?}", result.error);
        assert_eq!(
            result.approval_url,
            Some(format!(
                "https://test.example/approvals/{}",
                result.execution_id
            ))
        );
        result.execution_id
    }
    async fn decide_all(&self, id: Uuid, allow: bool) {
        let approvals = self.execution.approvals();
        let detail = approvals.detail(id).await.unwrap();
        approvals
            .decide(
                id,
                detail.pause_id,
                detail
                    .calls
                    .into_iter()
                    .filter(|c| c.error.is_none())
                    .map(|c| Decision {
                        call_id: c.call_id,
                        allow,
                    })
                    .collect(),
            )
            .await
            .unwrap();
    }
    async fn close(self) {
        self.stop.cancel();
        self.tasks.close();
        self.tasks.wait().await;
        self.sandbox.pool.close().await;
        self.db.close().await;
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn approval_batch_waits_restores_after_restart_and_never_replays_prior_calls(
    pool: sqlx::PgPool,
) {
    let f = Fixture::new(&pool).await;
    let id=f.pause("await tools.fixture.before()\nprint('before pause')\nvalues = await asyncio.gather(tools.fixture.read(), tools.fixture.write())\nprint('after pause')\nreturn values").await;
    assert_eq!(*f.hits.lock().unwrap(), ["before"]);
    let approvals = f.execution.approvals();
    let detail = approvals.detail(id).await.unwrap();
    assert_eq!(detail.calls.len(), 2);
    approvals
        .decide(
            id,
            detail.pause_id,
            vec![Decision {
                call_id: detail.calls[0].call_id,
                allow: true,
            }],
        )
        .await
        .unwrap();
    assert_eq!(
        f.execution
            .resume(f.principal.clone(), id)
            .await
            .unwrap()
            .status,
        "paused"
    );
    let (_, token) = f.auth.create("other", "test.example").await.unwrap();
    let other = f
        .auth
        .authenticate(&token, "test.example")
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        f.execution.resume(other, id).await,
        Err(Error::Approval(ApprovalError::NotFound))
    ));
    let mut wrong_host = f.principal.clone();
    wrong_host.audience = "other.example".into();
    assert!(f.execution.resume(wrong_host, id).await.is_err());
    approvals
        .decide(
            id,
            detail.pause_id,
            vec![Decision {
                call_id: detail.calls[1].call_id,
                allow: true,
            }],
        )
        .await
        .unwrap();
    f.sandbox.pool.close().await;
    let sandbox = Arc::new(
        MontySandbox::new(&binary(), Duration::from_secs(2), [73; 32], 32768)
            .await
            .unwrap(),
    );
    let restarted = Arc::new(Execution::new(
        f.db.clone(),
        f.sources.clone(),
        sandbox.clone(),
        f.tasks.clone(),
        f.stop.clone(),
        Default::default(),
        [73; 32],
    ));
    restarted.recover().await.unwrap();
    let result = restarted.resume(f.principal.clone(), id).await.unwrap();
    assert_eq!(result.status, "completed", "{:?}", result.error);
    assert_eq!(
        serde_json::from_str::<Value>(result.result.get()).unwrap(),
        json!(["read", "write"])
    );
    assert_eq!(result.output, "before pause\nafter pause\n");
    assert_eq!(result.tool_calls, 3);
    let mut hits = f.hits.lock().unwrap().clone();
    hits.sort();
    assert_eq!(hits, ["before", "read", "write"]);
    assert!(restarted.resume(f.principal.clone(), id).await.is_err());
    let outcomes: Vec<String> =
        sqlx::query_scalar("SELECT outcome FROM tool_calls WHERE execution_id=$1 ORDER BY seq")
            .bind(id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(outcomes, ["ok", "ok", "ok"]);
    assert!(approvals.list().await.unwrap().is_empty());
    let recorded:i64=sqlx::query_scalar("SELECT count(*) FROM approval_calls WHERE execution_id=$1 AND decision=true AND decided_at IS NOT NULL").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(recorded, 2);
    sandbox.pool.close().await;
    f.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn approval_denial_tamper_expiry_revocation_and_concurrent_resume_fail_closed(
    pool: sqlx::PgPool,
) {
    let f = Fixture::new(&pool).await;
    let id = f
        .pause("try:\n    await tools.fixture.write()\nexcept ToolError as e:\n    return str(e)")
        .await;
    f.decide_all(id, false).await;
    let result = f.execution.resume(f.principal.clone(), id).await.unwrap();
    assert_eq!(result.status, "completed");
    assert!(result.result.get().contains("denied by owner"));
    assert!(f.hits.lock().unwrap().is_empty());
    let id = f.pause("return await tools.fixture.write()").await;
    f.decide_all(id, true).await;
    let (a, b) = tokio::join!(
        f.execution.resume(f.principal.clone(), id),
        f.execution.resume(f.principal.clone(), id)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(*f.hits.lock().unwrap(), ["write"]);
    let id = f.pause("return await tools.fixture.write()").await;
    sqlx::query("UPDATE paused_runs SET context=overlay(context placing '\\x00'::bytea from 1 for 1) WHERE execution_id=$1").bind(id).execute(&pool).await.unwrap();
    assert!(matches!(
        f.execution.resume(f.principal.clone(), id).await,
        Err(Error::Approval(ApprovalError::Invalid))
    ));
    let id = f.pause("return await tools.fixture.write()").await;
    sqlx::query(
        "UPDATE paused_runs SET expires_at=now()-interval '1 second' WHERE execution_id=$1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        f.execution.resume(f.principal.clone(), id).await,
        Err(Error::Approval(ApprovalError::Expired))
    ));
    f.execution.approvals().sweep().await.unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM executions WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "expired");
    assert!(matches!(
        f.execution.resume(f.principal.clone(), id).await,
        Err(Error::Approval(ApprovalError::Expired))
    ));
    let id = f.pause("return await tools.fixture.write()").await;
    f.decide_all(id, true).await;
    f.auth.revoke(f.principal.token_id).await.unwrap();
    assert!(matches!(
        f.execution.resume(f.principal.clone(), id).await,
        Err(Error::Approval(ApprovalError::Expired))
    ));
    f.execution.approvals().sweep().await.unwrap();
    assert!(f.execution.approvals().list().await.unwrap().is_empty());
    assert_eq!(*f.hits.lock().unwrap(), ["write"]);
    f.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn policies_survive_sync_hide_blocked_tools_and_recheck_changed_operations(
    pool: sqlx::PgPool,
) {
    let f = Fixture::new(&pool).await;
    let policies = f.sources.policies();
    let policy = policies
        .list(f.endpoint, "", None)
        .await
        .unwrap()
        .tools
        .into_iter()
        .find(|p| p.upstream_name == "write")
        .unwrap();
    assert_eq!(policy.effective_action, Action::Approve);
    policies
        .set(
            f.endpoint,
            f.source,
            policy.source_revision,
            "write",
            Choice::Block,
        )
        .await
        .unwrap();
    assert!(
        f.catalogue
            .search_scoped("write", false, None, None, Some(f.principal.workspace_id))
            .await
            .unwrap()
            .tools
            .is_empty()
    );
    let direct = f
        .execution
        .execute(
            f.principal.clone(),
            "try:\n    await _call(\"fixture.write\", \"{}\")\nexcept ToolError as e:\n    return str(e)".into(),
        )
        .await
        .unwrap();
    assert_eq!(direct.status, "completed");
    assert!(direct.result.get().contains("unavailable"));
    assert!(f.hits.lock().unwrap().is_empty());
    policies
        .set(
            f.endpoint,
            f.source,
            policy.source_revision,
            "write",
            Choice::Default,
        )
        .await
        .unwrap();
    let id = f
        .pause("try:\n    await tools.fixture.write()\nexcept ToolError as e:\n    return str(e)")
        .await;
    f.decide_all(id, true).await;
    {
        let mut spec = f.spec.lock().unwrap();
        let mut operation = spec["paths"]["/write"].clone();
        operation["post"]["description"] = json!("changed target");
        spec["paths"].as_object_mut().unwrap().remove("/write");
        spec["paths"]["/other"] = operation;
    }
    f.sources.wake();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !f
            .sources
            .lookup("fixture.write")
            .await
            .is_some_and(|t| t.definition.description == "changed target")
        {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    let result = f.execution.resume(f.principal.clone(), id).await.unwrap();
    assert_eq!(result.status, "completed");
    assert!(result.result.get().contains("changed since approval"));
    assert!(f.hits.lock().unwrap().is_empty());
    policies
        .set(
            f.endpoint,
            f.source,
            policy.source_revision,
            "write",
            Choice::Block,
        )
        .await
        .unwrap();
    f.sources.resync(f.source).await.unwrap();
    assert_eq!(
        policies
            .list(f.endpoint, "", None)
            .await
            .unwrap()
            .tools
            .into_iter()
            .find(|p| p.upstream_name == "write")
            .unwrap()
            .effective_action,
        Action::Block
    );
    assert!(
        policies
            .set(
                f.endpoint,
                f.source,
                policy.source_revision,
                "write",
                Choice::Allow
            )
            .await
            .is_err()
    );
    f.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn repeated_pauses_keep_decisions_separate_and_call_limits_cumulative(pool: sqlx::PgPool) {
    let f = Fixture::new(&pool).await;
    let id=f.pause("await tools.fixture.write()\ntry:\n    await tools.fixture.write()\nexcept ToolError as e:\n    return str(e)").await;
    let first = f.execution.approvals().detail(id).await.unwrap();
    f.decide_all(id, true).await;
    let result = f.execution.resume(f.principal.clone(), id).await.unwrap();
    assert_eq!(result.status, "paused");
    assert_eq!(result.tool_calls, 2);
    assert_eq!(*f.hits.lock().unwrap(), ["write"]);
    let second = f.execution.approvals().detail(id).await.unwrap();
    assert_ne!(first.pause_id, second.pause_id);
    assert!(matches!(
        f.execution
            .approvals()
            .decide(
                id,
                first.pause_id,
                vec![Decision {
                    call_id: first.calls[0].call_id,
                    allow: true
                }]
            )
            .await,
        Err(ApprovalError::Conflict)
    ));
    f.decide_all(id, false).await;
    let result = f.execution.resume(f.principal.clone(), id).await.unwrap();
    assert_eq!(result.status, "completed");
    assert!(result.result.get().contains("denied by owner"));
    let decisions: Vec<bool> = sqlx::query_scalar(
        "SELECT decision FROM approval_calls WHERE execution_id=$1 ORDER BY decided_at",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(decisions, [true, false]);
    let id=f.pause("await asyncio.gather(*[tools.fixture.read() for i in range(99)])\nawait tools.fixture.write()\nreturn await tools.fixture.write()").await;
    f.decide_all(id, true).await;
    let result = f.execution.resume(f.principal.clone(), id).await.unwrap();
    assert_eq!(result.status, "failed");
    assert!(result.error.as_deref().unwrap().contains("100"));
    assert_eq!(result.tool_calls, 100);
    assert_eq!(
        f.hits
            .lock()
            .unwrap()
            .iter()
            .filter(|hit| **hit == "write")
            .count(),
        2
    );
    f.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn tool_allowlists_override_allow_policies_survive_sync_and_recheck_paused_calls(
    pool: sqlx::PgPool,
) {
    let f = Fixture::new(&pool).await;
    let endpoints = Workspaces::new(f.db.clone(), "test.example".into()).unwrap();
    let policies = f.sources.policies();
    let revision = f.sources.get(f.source).await.unwrap().unwrap().revision;
    policies
        .set(f.endpoint, f.source, revision, "write", Choice::Allow)
        .await
        .unwrap();
    endpoints.set_all_tools(f.endpoint, false).await.unwrap();
    assert!(!endpoints.get(f.endpoint).await.unwrap().unwrap().all_tools);
    assert!(
        f.catalogue
            .search_scoped("", false, None, None, Some(f.principal.workspace_id))
            .await
            .unwrap()
            .tools
            .is_empty()
    );
    let result=f.execution.execute(f.principal.clone(),"try:\n    await _call('fixture.write', '{}')\nexcept ToolError as e:\n    return str(e)".into()).await.unwrap();
    assert_eq!(result.status, "completed");
    assert!(result.result.get().contains("unavailable"));
    assert!(f.hits.lock().unwrap().is_empty());
    policies
        .set_in_scope(f.endpoint, f.source, revision, "read", true)
        .await
        .unwrap();
    let page = f
        .catalogue
        .search_scoped("", false, None, None, Some(f.principal.workspace_id))
        .await
        .unwrap();
    assert_eq!(page.tools.len(), 1);
    assert_eq!(page.tools[0].path, "fixture.read");
    let result = f
        .execution
        .execute(
            f.principal.clone(),
            "return await tools.fixture.read()".into(),
        )
        .await
        .unwrap();
    assert_eq!(result.status, "completed");
    assert_eq!(*f.hits.lock().unwrap(), ["read"]);
    f.sources.resync(f.source).await.unwrap();
    let updated = f.sources.get(f.source).await.unwrap().unwrap().revision;
    tokio::time::timeout(Duration::from_secs(1), async {
        while !f
            .sources
            .lookup("fixture.write")
            .await
            .is_some_and(|t| t.revision == updated)
        {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    assert_eq!(
        f.catalogue
            .search_scoped("", false, None, None, Some(f.principal.workspace_id))
            .await
            .unwrap()
            .tools
            .len(),
        1
    );
    assert!(
        policies
            .set_in_scope(f.endpoint, f.source, revision, "write", true)
            .await
            .is_err()
    );
    let empty = endpoints.create("empty", &[]).await.unwrap();
    assert!(
        policies
            .set_in_scope(empty, f.source, updated, "read", true)
            .await
            .is_err()
    );
    policies
        .set_in_scope(f.endpoint, f.source, updated, "write", true)
        .await
        .unwrap();
    policies
        .set(f.endpoint, f.source, updated, "write", Choice::Default)
        .await
        .unwrap();
    let id = f
        .pause("try:\n    await tools.fixture.write()\nexcept ToolError as e:\n    return str(e)")
        .await;
    f.decide_all(id, true).await;
    policies
        .set_in_scope(f.endpoint, f.source, updated, "write", false)
        .await
        .unwrap();
    let resumed = f.execution.resume(f.principal.clone(), id).await.unwrap();
    assert_eq!(resumed.status, "completed");
    assert!(resumed.result.get().contains("unavailable"));
    assert_eq!(*f.hits.lock().unwrap(), ["read"]);
    let mut source = f.sources.get(f.source).await.unwrap().unwrap().source;
    let Transport::OpenApi { server_url, .. } = &mut source.transport else {
        panic!()
    };
    *server_url = Some("http://changed.example".into());
    f.sources.save(Some(f.source), source).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM endpoint_tools WHERE source_id=$1")
        .bind(f.source)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    f.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn saved_script_execution_keeps_scope_policy_audit_and_paused_code(pool: sqlx::PgPool) {
    use codemode_core::components::scripts::{Content, Scripts};
    let f = Fixture::new(&pool).await;
    let scripts = Scripts::new(f.db.clone());
    let code = "await tools.fixture.before()\nreturn await tools.fixture.write()";
    let script = scripts
        .save(
            f.endpoint,
            None,
            Content {
                name: "Saved workflow".into(),
                description: "".into(),
                code: code.into(),
            },
        )
        .await
        .unwrap();
    let id = script.summary.id;
    let paused = f
        .execution
        .execute_script(f.principal.clone(), id)
        .await
        .unwrap();
    assert_eq!(paused.status, "paused");
    assert_eq!(*f.hits.lock().unwrap(), ["before"]);
    scripts
        .save(
            f.endpoint,
            Some((id, 1)),
            Content {
                name: "Saved workflow".into(),
                description: "".into(),
                code: "return 'changed'".into(),
            },
        )
        .await
        .unwrap();
    scripts.remove(f.endpoint, id, 2).await.unwrap();
    f.decide_all(paused.execution_id, true).await;
    let result = f
        .execution
        .resume(f.principal.clone(), paused.execution_id)
        .await
        .unwrap();
    assert_eq!(result.status, "completed");
    assert_eq!(*f.hits.lock().unwrap(), ["before", "write"]);
    let audited: String = sqlx::query_scalar("SELECT code FROM executions WHERE id=$1")
        .bind(paused.execution_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(audited, code);
    let script = scripts
        .save(
            f.endpoint,
            None,
            Content {
                name: "Read".into(),
                description: "".into(),
                code: "return await tools.fixture.read()".into(),
            },
        )
        .await
        .unwrap();
    let mut foreign = f.principal.clone();
    foreign.workspace_id = Uuid::new_v4();
    assert!(matches!(
        f.execution.execute_script(foreign, script.summary.id).await,
        Err(Error::Script(_))
    ));
    let read = f
        .execution
        .execute_script(f.principal.clone(), script.summary.id)
        .await
        .unwrap();
    assert_eq!(read.status, "completed");
    let policies = f.sources.policies();
    let revision = f.sources.get(f.source).await.unwrap().unwrap().revision;
    policies
        .set(f.endpoint, f.source, revision, "read", Choice::Block)
        .await
        .unwrap();
    let blocked = f
        .execution
        .execute_script(f.principal.clone(), script.summary.id)
        .await
        .unwrap();
    assert_eq!(blocked.status, "failed");
    policies
        .set(f.endpoint, f.source, revision, "read", Choice::Allow)
        .await
        .unwrap();
    let endpoints = Workspaces::new(f.db.clone(), "test.example".into()).unwrap();
    endpoints.set_all_tools(f.endpoint, false).await.unwrap();
    let outside_scope = f
        .execution
        .execute_script(f.principal.clone(), script.summary.id)
        .await
        .unwrap();
    assert_eq!(outside_scope.status, "failed");
    assert_eq!(*f.hits.lock().unwrap(), ["before", "write", "read"]);
    f.close().await;
}
