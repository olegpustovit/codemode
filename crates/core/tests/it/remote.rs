use super::*;
use axum::Router;
use codemode_core::components::sources::{Config, Source, Transport};
use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};

#[derive(Clone)]
pub(super) struct Upstream;
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct EchoArgs {
    value: String,
    #[serde(default)]
    prefix: String,
}
#[tool_router]
impl Upstream {
    #[tool(description = "Deliberate fixture failure")]
    async fn fail(&self) -> CallToolResult {
        CallToolResult::error(vec![ContentBlock::text("deliberate upstream failure")])
    }

    #[tool(description = "Echo a value", annotations(read_only_hint = true))]
    async fn echo(&self, Parameters(args): Parameters<EchoArgs>) -> CallToolResult {
        CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string(&format!("{}{}", args.prefix, args.value)).unwrap(),
        )])
    }
}
#[tool_handler]
impl ServerHandler for Upstream {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn remote_source_sync_dispatch_and_audit(pool: sqlx::PgPool) {
    let stop = CancellationToken::new();
    let tasks = TaskTracker::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let service = StreamableHttpService::new(
        || Ok(Upstream),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true),
    );
    let router = Router::new().nest_service("/mcp", service);
    let server_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let db = database(&pool).await;
    let auth = Auth::new(db.clone());
    let (_, token) = auth.create("test", "test.example").await.unwrap();
    let principal = auth
        .authenticate(&token, "test.example")
        .await
        .unwrap()
        .unwrap();
    let catalogue = Catalogue::new(db.clone());
    let sources = managed_sources(db.clone(), catalogue.clone());
    let sync = sources.clone();
    let sync_stop = stop.clone();
    sync.import(Config {
        sources: vec![
            Source {
                requires_account: false,
                slug: "broken".into(),
                transport: Transport::McpHttp {
                    url: "http://127.0.0.1:1/mcp".into(),
                    bearer_env: None,
                },
            },
            Source {
                requires_account: false,
                slug: "fixture".into(),
                transport: Transport::McpHttp {
                    url: format!("http://{address}/mcp"),
                    bearer_env: None,
                },
            },
            Source {
                requires_account: false,
                slug: "search".into(),
                transport: Transport::McpHttp {
                    url: format!("http://{address}/mcp"),
                    bearer_env: None,
                },
            },
        ],
    })
    .await
    .unwrap();
    tasks.spawn(async move {
        sync.run(sync_stop).await;
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let broken: String =
                sqlx::query_scalar("SELECT status FROM sources WHERE slug='broken'")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            if sources.lookup("fixture.echo").await.is_some()
                && sources.lookup("search.echo").await.is_some()
                && broken != "pending"
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("fixture source should synchronize");
    let broken: String = sqlx::query_scalar("SELECT status FROM sources WHERE slug='broken'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(broken, "error");
    assert_eq!(
        catalogue
            .search("echo", false, None, None)
            .await
            .unwrap()
            .tools
            .len(),
        2
    );
    let binary = std::env::var_os("CODEMODE_MONTY_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.tools/monty/bin/monty")
        });
    let sandbox = Arc::new(
        codemode_core::infrastructure::sandbox::MontySandbox::new(
            &binary,
            std::time::Duration::from_secs(1),
            32768,
        )
        .await
        .unwrap(),
    );
    let execution = Arc::new(Execution::new(
        db.clone(),
        sources,
        sandbox.clone(),
        tasks.clone(),
        stop.clone(),
        Default::default(),
    ));
    let discovered = execution.execute(principal.clone(), "hits = await tools.search(query='echo')\nprint(help(tools.fixture.echo))\nreturn [hits[0]['path'], await tools.fixture.echo(value='inline'), await tools.search.echo(value='collision')]".into()).await.unwrap();
    assert_eq!(discovered.status, "completed", "{:?}", discovered.error);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(discovered.result.get()).unwrap(),
        serde_json::json!(["fixture.echo", "inline", "collision"])
    );
    assert_eq!(discovered.tool_calls, 3);
    assert!(discovered.output.contains("async def tools.fixture.echo"));
    let discovery_audit: Vec<(String, String)> =
        sqlx::query_as("SELECT path,outcome FROM tool_calls WHERE execution_id=$1 ORDER BY seq")
            .bind(discovered.execution_id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        discovery_audit,
        vec![
            ("@codemode/search".into(), "ok".into()),
            ("fixture.echo".into(), "ok".into()),
            ("search.echo".into(), "ok".into())
        ]
    );
    let workspaces =
        codemode_core::components::workspaces::Workspaces::new(db.clone(), "test.example".into())
            .unwrap();
    let empty = workspaces.create("Empty", &[]).await.unwrap();
    let (_, empty_token) = auth
        .create_for_workspace("empty", empty, "test.example", "/mcp")
        .await
        .unwrap()
        .unwrap();
    let empty_principal = auth
        .authenticate(&empty_token, "test.example")
        .await
        .unwrap()
        .unwrap();
    let hidden = execution
        .execute(empty_principal, "return await tools.search('echo')".into())
        .await
        .unwrap();
    assert_eq!(hidden.status, "completed", "{:?}", hidden.error);
    assert_eq!(hidden.result.get(), "[]");
    let cap = execution.execute(principal.clone(), "n = 0\ntry:\n    for _ in range(101):\n        await tools.search('echo')\n        n += 1\nexcept ToolError as e:\n    return [n, str(e)]".into()).await.unwrap();
    assert_eq!(cap.status, "completed", "{:?}", cap.error);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(cap.result.get()).unwrap(),
        serde_json::json!([100, "tool-call limit (100) reached"])
    );
    assert_eq!(cap.tool_calls, 100);
    let result = execution
        .execute(
            principal.clone(),
            r#"return await tools.fixture.echo(value="hello", prefix=None)"#.into(),
        )
        .await
        .unwrap();
    assert_eq!(result.status, "completed");
    assert_eq!(result.result.get(), r#""hello""#);
    let audit: (String, String) =
        sqlx::query_as("SELECT path,outcome FROM tool_calls WHERE execution_id=$1")
            .bind(result.execution_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(audit, ("fixture.echo".into(), "ok".into()));
    let unknown = execution
        .execute(
            principal.clone(),
            "return await tools.fixture.ech(value='hello')".into(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.status, "failed");
    assert!(
        unknown
            .error
            .as_deref()
            .unwrap()
            .contains("did you mean: fixture.echo, fixture.fail"),
        "{:?}",
        unknown.error
    );
    for code in [
        "try:\n    await tools.fixture.ech(value='hello')\nexcept ToolError as e:\n    return str(e)",
        "try:\n    missing = tools.fixture.ech\nexcept ToolError as e:\n    return str(e)",
        "try:\n    await tools.search.ech(value='hello')\nexcept ToolError as e:\n    return str(e)",
        "try:\n    await tools.typo.echo()\nexcept ToolError as e:\n    return str(e)",
    ] {
        let caught = execution
            .execute(principal.clone(), code.into())
            .await
            .unwrap();
        assert_eq!(caught.status, "completed", "{:?}", caught.error);
        let message = serde_json::from_str::<String>(caught.result.get()).unwrap();
        assert!(message.contains("did you mean:"), "{message}");
        assert!(
            message
                .split("did you mean: ")
                .nth(1)
                .unwrap()
                .split(", ")
                .count()
                <= 3
        );
        assert_eq!(caught.tool_calls, 1);
    }
    let namespace = execution
        .execute(
            principal.clone(),
            "try:\n    await tools.fixture()\nexcept TypeError as e:\n    return str(e)".into(),
        )
        .await
        .unwrap();
    assert_eq!(namespace.status, "completed", "{:?}", namespace.error);
    assert!(namespace.result.get().contains("namespace"));
    assert_eq!(namespace.tool_calls, 0);
    for (code, status, expected) in [
        (
            r#"try:
    await tools.fixture.fail()
except ToolError as e:
    return str(e)"#,
            "completed",
            Some("fixture.fail: upstream tool failed: deliberate upstream failure"),
        ),
        (
            r#"try:
    await tools.fixture.echo(value=42)
except ToolError as e:
    return str(e)"#,
            "completed",
            Some(
                "fixture.echo: invalid argument at /value (expected string)\n  signature: async def tools.fixture.echo(*, value: str, prefix: str = None): ...\n# ",
            ),
        ),
        (
            r#"try:
    await _call("missing.tool", "{}")
except ToolError:
    return "unknown tool caught""#,
            "completed",
            Some("unknown tool caught"),
        ),
        ("raise ValueError('uncaught failure')", "failed", None),
    ] {
        let result = execution
            .execute(principal.clone(), code.into())
            .await
            .unwrap();
        assert_eq!(result.status, status, "{:?}", result.error);
        if let Some(expected) = expected {
            assert_eq!(
                serde_json::from_str::<String>(result.result.get()).unwrap(),
                expected
            );
        }
    }
    let large = execution
        .execute(principal, "return 'x' * (5 * 1024 * 1024)".into())
        .await
        .unwrap();
    assert_eq!(large.status, "completed", "{:?}", large.error);
    assert!(large.result_truncated);
    assert!(large.result.get().len() <= 32768);
    assert!(large.result_dropped_bytes > 5 * 1024 * 1024 - 32768);
    let outcomes: Vec<(String, i64)> =
        sqlx::query_as("SELECT outcome,count(*) FROM tool_calls GROUP BY outcome ORDER BY outcome")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        outcomes,
        vec![
            ("invalid_args".into(), 6),
            ("ok".into(), 105),
            ("tool_error".into(), 2)
        ]
    );
    stop.cancel();
    tasks.close();
    tasks.wait().await;
    sandbox.pool.close().await;
}
