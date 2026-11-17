use super::*;
use axum::{
    Json, Router,
    routing::{get, post},
};
use codemode_core::{
    components::{
        auth::Principal,
        policies::{Action, Choice},
        sources::{Source, Transport},
        workspaces::Workspaces,
    },
    infrastructure::sandbox::MontySandbox,
};
use serde_json::json;
use std::{sync::Mutex, time::Duration};

struct Fixture {
    db: Database,
    sources: Sources,
    catalogue: Catalogue,
    sandbox: Arc<MontySandbox>,
    execution: Arc<Execution<MontySandbox>>,
    principal: Principal,
    endpoint: Uuid,
    source: Uuid,
    hits: Arc<Mutex<Vec<&'static str>>>,
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
                "/large":{"get":{"operationId":"large","responses":{}}},
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
        router = router.route("/large", get(|| async { Json("雪".repeat(6000)) }));
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
            MontySandbox::new(&binary(), Duration::from_secs(2), 32768)
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
            endpoint,
            source,
            hits,
            tasks,
            stop,
        }
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
async fn default_allow_and_blocked_python_permission_error_are_audited(pool: sqlx::PgPool) {
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
    assert_eq!(policy.effective_action, Action::Allow);
    assert!(matches!(
        policies.effective(Uuid::new_v4(), f.source, "write").await,
        Err(codemode_core::components::policies::Error::NotFound)
    ));
    let completed = f
        .execution
        .execute(
            f.principal.clone(),
            "return await asyncio.gather(tools.fixture.read(), tools.fixture.write())".into(),
        )
        .await
        .unwrap();
    assert_eq!(completed.status, "completed");
    assert_eq!(completed.tool_calls, 2);
    assert_eq!(f.hits.lock().unwrap().len(), 2);
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
    let blocked_hits = f
        .catalogue
        .search_scoped("write", false, None, None, Some(f.endpoint))
        .await
        .unwrap();
    assert_eq!(blocked_hits.tools.len(), 1);
    assert!(blocked_hits.tools[0].blocked);
    assert_eq!(blocked_hits.tools[0].path, "fixture.write");
    let blocked=f.execution.execute(f.principal.clone(),"try:\n    await tools.fixture.write()\nexcept PermissionError as e:\n    return str(e)".into()).await.unwrap();
    assert_eq!(blocked.status, "completed", "{:?}", blocked.error);
    assert!(blocked.result.get().contains("fixture.write"));
    assert!(blocked.result.get().contains(&f.endpoint.to_string()));
    assert_eq!(f.hits.lock().unwrap().len(), 2);
    let outcome: String =
        sqlx::query_scalar("SELECT outcome FROM tool_calls WHERE execution_id=$1")
            .bind(blocked.execution_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(outcome, "blocked");
    f.sources.resync(f.source).await.unwrap();
    let revised = f.sources.get(f.source).await.unwrap().unwrap().revision;
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
    assert_eq!(
        policies
            .list(f.endpoint, "write", None)
            .await
            .unwrap()
            .tools[0]
            .effective_action,
        Action::Block
    );
    policies
        .set(f.endpoint, f.source, revised, "write", Choice::Default)
        .await
        .unwrap();
    assert_eq!(
        policies
            .list(f.endpoint, "write", None)
            .await
            .unwrap()
            .tools[0]
            .effective_action,
        Action::Allow
    );
    f.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn tool_allowlists_override_allow_policies_survive_sync_and_identity_changes(
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
    let hidden_help = f
        .execution
        .execute(
            f.principal.clone(),
            "return help(tools.fixture.write)".into(),
        )
        .await
        .unwrap();
    assert_eq!(hidden_help.status, "completed", "{:?}", hidden_help.error);
    assert_eq!(
        serde_json::from_str::<String>(hidden_help.result.get()).unwrap(),
        "tool is outside workspace tool scope"
    );
    assert_eq!(hidden_help.tool_calls, 0);
    assert!(
        f.catalogue
            .search_scoped("", false, None, None, Some(f.principal.workspace_id))
            .await
            .unwrap()
            .tools
            .is_empty()
    );
    let result=f.execution.execute(f.principal.clone(),"try:\n    await _call('fixture.write', '{}')\nexcept PermissionError as e:\n    return str(e)".into()).await.unwrap();
    assert_eq!(result.status, "completed");
    assert!(result.result.get().contains("blocked"));
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
    policies
        .set_in_scope(f.endpoint, f.source, updated, "write", false)
        .await
        .unwrap();
    let blocked=f.execution.execute(f.principal.clone(),"try:\n    await tools.fixture.write()\nexcept PermissionError as e:\n    return str(e)".into()).await.unwrap();
    assert_eq!(blocked.status, "completed");
    assert!(blocked.result.get().contains("blocked"));
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
async fn saved_script_execution_keeps_scope_policy_and_audit(pool: sqlx::PgPool) {
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
    let result = f
        .execution
        .execute_script(f.principal.clone(), id, Default::default())
        .await
        .unwrap();
    assert_eq!(result.status, "completed");
    assert_eq!(*f.hits.lock().unwrap(), ["before", "write"]);
    let audited: String = sqlx::query_scalar("SELECT code FROM executions WHERE id=$1")
        .bind(result.execution_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(audited, code);
    scripts.remove(f.endpoint, id, 1).await.unwrap();
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
        f.execution
            .execute_script(foreign, script.summary.id, Default::default())
            .await,
        Err(Error::Script(_))
    ));
    let read = f
        .execution
        .execute_script(f.principal.clone(), script.summary.id, Default::default())
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
        .execute_script(f.principal.clone(), script.summary.id, Default::default())
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
        .execute_script(f.principal.clone(), script.summary.id, Default::default())
        .await
        .unwrap();
    assert_eq!(outside_scope.status, "failed");
    assert_eq!(*f.hits.lock().unwrap(), ["before", "write", "read"]);
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn approval_removal_migrates_policies_and_preserves_history_without_replay(
    pool: sqlx::PgPool,
) {
    for migration in sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version <= 16)
    {
        sqlx::raw_sql(migration.sql.clone())
            .execute(&pool)
            .await
            .unwrap();
    }
    let source: Uuid = sqlx::query_scalar(
        "INSERT INTO sources(slug,kind,config) VALUES('legacy','mcp_http','{}') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let workspace:Uuid=sqlx::query_scalar("INSERT INTO endpoints(slug,hostname,all_sources) VALUES('Legacy workspace','test.example',true) RETURNING id").fetch_one(&pool).await.unwrap();
    let token:Uuid=sqlx::query_scalar("INSERT INTO access_tokens(name,audience,token_hash,workspace_id) VALUES('Legacy token','test.example',decode(repeat('01',32),'hex'),$1) RETURNING id").bind(workspace).fetch_one(&pool).await.unwrap();
    for (name, action) in [("write", "approve"), ("read", "allow"), ("secret", "block")] {
        sqlx::query("INSERT INTO tool_policies(endpoint_id,source_id,upstream_name,action) VALUES($1,$2,$3,$4)").bind(workspace).bind(source).bind(name).bind(action).execute(&pool).await.unwrap();
    }
    let execution:Uuid=sqlx::query_scalar("INSERT INTO executions(token_id,code,status) VALUES($1,'legacy side effects','paused') RETURNING id").bind(token).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO tool_calls(execution_id,seq,path,args_redacted,outcome) VALUES($1,0,'legacy.write','{}','awaiting_approval')").bind(execution).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO paused_runs(execution_id,pause_id,token_id,audience,expires_at,context,snapshot,signature) VALUES($1,$2,$3,'test.example',now()+interval '1 minute','\\x00','\\x00',decode(repeat('00',32),'hex'))").bind(execution).bind(Uuid::new_v4()).bind(token).execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0018_remove_approvals.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM tool_policies ORDER BY upstream_name")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(actions, ["allow", "block", "allow"]);
    let row: (String, String, bool) =
        sqlx::query_as("SELECT status,code,finished_at IS NOT NULL FROM executions WHERE id=$1")
            .bind(execution)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row, ("failed".into(), "legacy side effects".into(), true));
    let outcome: String =
        sqlx::query_scalar("SELECT outcome FROM tool_calls WHERE execution_id=$1")
            .bind(execution)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(outcome, "cancelled");
    let removed: bool = sqlx::query_scalar(
        "SELECT to_regclass('paused_runs') IS NULL AND to_regclass('approval_calls') IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(removed);
    assert!(
        sqlx::query("UPDATE tool_policies SET action='approve'")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE tool_policies SET action='ask'")
            .execute(&pool)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn session_history_stores_bounded_utf8_results_previews_and_stdout_on_failure(
    pool: sqlx::PgPool,
) {
    use codemode_core::components::{agent_sessions::AgentSessions, execution::log::Log};
    let f = Fixture::new(&pool).await;
    let result = f
        .execution
        .execute(
            f.principal.clone(),
            "# Large result\nprint('雪' * 6000)\nreturn await tools.fixture.large()".into(),
        )
        .await
        .unwrap();
    assert_eq!(result.status, "completed", "{:?}", result.error);
    assert!(result.result.get().len() > 16384);
    let history = AgentSessions::new(f.db.clone());
    let page = history.list("Large result", None).await.unwrap();
    assert_eq!(page.sessions.len(), 1);
    assert_eq!(page.sessions[0].sources, ["fixture"]);
    let detail = history.detail(page.sessions[0].id, None).await.unwrap();
    let stored = &detail.executions[0];
    assert_eq!(stored.summary.title.as_deref(), Some("Large result"));
    for value in [&stored.result, &stored.output] {
        let text = value.as_deref().unwrap();
        assert!(text.len() <= 16384);
        assert!(text.ends_with('…'));
    }
    let preview = stored.calls[0].preview.as_deref().unwrap();
    assert!(preview.len() <= 2048);
    assert!(preview.ends_with('…'));
    let failed=f.execution.execute(f.principal.clone(),"print('before tool')\nawait tools.fixture.read()\nprint('after tool')\nraise ValueError('fixture failure')".into()).await.unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.output, "before tool\nafter tool\n");
    let stored = Log::new(f.db.clone())
        .detail(failed.execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.output.as_deref(), Some("before tool\nafter tool\n"));
    assert_eq!(stored.summary.title.as_deref(), Some("fixture.read"));
    let timeout = f
        .execution
        .execute(
            f.principal.clone(),
            "print('before deadline')\nwhile True: pass".into(),
        )
        .await
        .unwrap();
    assert_eq!(timeout.status, "failed");
    assert_eq!(timeout.output, "before deadline\n");
    let stored = Log::new(f.db.clone())
        .detail(timeout.execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.output.as_deref(), Some("before deadline\n"));
    f.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn source_policy_updates_all_tools_atomically_and_rejects_stale_or_excluded_sources(
    pool: sqlx::PgPool,
) {
    let f = Fixture::new(&pool).await;
    let policies = f.sources.policies();
    let tools = policies.list(f.endpoint, "", None).await.unwrap().tools;
    assert_eq!(tools.len(), 4);
    assert!(tools.iter().all(|t| t.signature.contains("async def")));
    let revision = tools[0].source_revision;
    policies
        .set_source(f.endpoint, f.source, revision, Action::Block)
        .await
        .unwrap();
    assert!(
        policies
            .list(f.endpoint, "", None)
            .await
            .unwrap()
            .tools
            .iter()
            .all(|t| t.effective_action == Action::Block)
    );
    assert!(
        policies
            .set_source(f.endpoint, f.source, revision + 1, Action::Allow)
            .await
            .is_err()
    );
    assert!(
        policies
            .list(f.endpoint, "", None)
            .await
            .unwrap()
            .tools
            .iter()
            .all(|t| t.effective_action == Action::Block)
    );
    let empty = Workspaces::new(f.db.clone(), "test.example".into())
        .unwrap()
        .create("Empty", &[])
        .await
        .unwrap();
    assert!(
        policies
            .set_source(empty, f.source, revision, Action::Allow)
            .await
            .is_err()
    );
    policies
        .set_source(f.endpoint, f.source, revision, Action::Allow)
        .await
        .unwrap();
    assert!(
        policies
            .list(f.endpoint, "", None)
            .await
            .unwrap()
            .tools
            .iter()
            .all(|t| t.effective_action == Action::Allow)
    );
    f.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn saved_script_args_are_data_and_have_their_own_bound(pool: sqlx::PgPool) {
    use codemode_core::components::scripts::{Content, Scripts};
    let f = Fixture::new(&pool).await;
    let script = Scripts::new(f.db.clone())
        .save(
            f.endpoint,
            None,
            Content {
                name: "Arguments".into(),
                description: "".into(),
                code: "return args".into(),
            },
        )
        .await
        .unwrap();
    let input =
        serde_json::json!({"text":"'); await tools.fixture.write() #", "nested":{"number":42}});
    let result = f
        .execution
        .execute_script(
            f.principal.clone(),
            script.summary.id,
            input.as_object().unwrap().clone(),
        )
        .await
        .unwrap();
    assert_eq!(result.status, "completed", "{:?}", result.error);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(result.result.get()).unwrap(),
        input
    );
    assert_eq!(result.tool_calls, 0);
    assert!(f.hits.lock().unwrap().is_empty());
    let large = serde_json::json!({"value":"x".repeat(65536)})
        .as_object()
        .unwrap()
        .clone();
    assert!(matches!(
        f.execution
            .execute_script(f.principal.clone(), script.summary.id, large)
            .await,
        Err(codemode_core::components::execution::Error::Args)
    ));
    f.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn session_memo_persists_atomic_bounded_non_sensitive_json_and_expires(pool: sqlx::PgPool) {
    use codemode_core::components::agent_sessions::{
        AgentSessions, Error as SessionError, memo::Request,
    };
    let f = Fixture::new(&pool).await;
    let first = f
        .execution
        .execute(
            f.principal.clone(),
            "memo.set('team', {'id': 'EVL'})\nraise ValueError('later failure')".into(),
        )
        .await
        .unwrap();
    assert_eq!(first.status, "failed");
    let next = f.execution.execute(f.principal.clone(), "return [memo.get('team'), memo.get('missing', 'fallback'), memo.set('empty', None), memo.get('empty', 'fallback')]".into()).await.unwrap();
    assert_eq!(next.status, "completed", "{:?}", next.error);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(next.result.get()).unwrap(),
        json!([{"id":"EVL"},"fallback",null,null])
    );
    let audit: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT args_redacted::text,preview FROM tool_calls WHERE execution_id=$1")
            .bind(next.execution_id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(audit.iter().all(|(args, preview)| args.contains("REDACTED")
        && !args.contains("team")
        && preview.is_none()));
    let denied = f.execution.execute(f.principal.clone(), "try:\n    memo.set('api_key', 'rejected-memo-fixture')\nexcept ToolError as e:\n    return str(e)".into()).await.unwrap();
    assert_eq!(denied.status, "completed", "{:?}", denied.error);
    assert!(denied.result.get().contains("non-sensitive"));
    let recorded: String =
        sqlx::query_scalar("SELECT args_redacted::text FROM tool_calls WHERE execution_id=$1")
            .bind(denied.execution_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!recorded.contains("rejected-memo-fixture"));
    let service = AgentSessions::new(f.db.clone());
    let (a, b) = tokio::join!(
        service.memo(
            next.execution_id,
            f.endpoint,
            Request::Set {
                key: "a".into(),
                value: json!(1)
            }
        ),
        service.memo(
            next.execution_id,
            f.endpoint,
            Request::Set {
                key: "b".into(),
                value: json!(2)
            }
        )
    );
    assert_eq!(a.unwrap(), json!(1));
    assert_eq!(b.unwrap(), json!(2));
    for (key, value) in [("a", json!(1)), ("b", json!(2))] {
        assert_eq!(
            service
                .memo(
                    next.execution_id,
                    f.endpoint,
                    Request::Get { key: key.into() }
                )
                .await
                .unwrap()["value"],
            value
        );
    }
    service
        .memo(
            next.execution_id,
            f.endpoint,
            Request::Set {
                key: "large".into(),
                value: json!("x".repeat(40000)),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        service
            .memo(
                next.execution_id,
                f.endpoint,
                Request::Set {
                    key: "overflow".into(),
                    value: json!("y".repeat(30000))
                }
            )
            .await,
        Err(SessionError::MemoLimit)
    ));
    assert_eq!(
        service
            .memo(
                next.execution_id,
                f.endpoint,
                Request::Get {
                    key: "overflow".into()
                }
            )
            .await
            .unwrap()["found"],
        false
    );
    let (_, other_token) = Auth::new(f.db.clone())
        .create("other agent", "test.example")
        .await
        .unwrap();
    let other = Auth::new(f.db.clone())
        .authenticate(&other_token, "test.example")
        .await
        .unwrap()
        .unwrap();
    let isolated = f
        .execution
        .execute(other, "return memo.get('team', 'isolated')".into())
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<String>(isolated.result.get()).unwrap(),
        "isolated"
    );
    sqlx::query("UPDATE agent_sessions SET last_seen_at=now()-interval '5 days' WHERE id=(SELECT session_id FROM executions WHERE id=$1)").bind(next.execution_id).execute(&pool).await.unwrap();
    assert!(matches!(
        service
            .memo(
                next.execution_id,
                f.endpoint,
                Request::Get { key: "team".into() }
            )
            .await,
        Err(SessionError::NotFound)
    ));
    service.sweep().await.unwrap();
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_session_memo")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
    f.close().await;
}
