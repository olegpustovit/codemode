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
            if sources.lookup("fixture.echo").await.is_some() && broken != "pending" {
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
        1
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
        db,
        sources,
        sandbox.clone(),
        tasks.clone(),
        stop.clone(),
        Default::default(),
    ));
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
            "try:\n    await tools.fixture.ech(value='hello')\nexcept ToolError as e:\n    return str(e)".into(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.status, "completed");
    assert!(
        serde_json::from_str::<String>(unknown.result.get())
            .unwrap()
            .contains("did you mean: fixture.echo, fixture.fail")
    );
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
                "fixture.echo: invalid argument at /value (expected string)\n  signature: async def tools.fixture.echo(*, value: str, prefix: str = None) -> Any: ...\n# ",
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
        .execute(principal.clone(), "return 'x' * (5 * 1024 * 1024)".into())
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
            ("invalid_args".into(), 1),
            ("ok".into(), 1),
            ("tool_error".into(), 3)
        ]
    );
    let budget = execution
        .execute(
            principal,
            "for i in range(100):\n    await tools.fixture.echo(value='x')\nreturn 'completed'"
                .into(),
        )
        .await
        .unwrap();
    assert_eq!(budget.status, "failed");
    assert!(
        budget
            .error
            .as_deref()
            .unwrap()
            .contains("suspension limit")
    );
    assert_eq!(budget.tool_calls, 55, "{:?}", budget.error);
    stop.cancel();
    tasks.close();
    tasks.wait().await;
    sandbox.pool.close().await;
}
