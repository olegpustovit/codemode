use super::server;
use axum::Router;
use codemode_core::{
    components::{
        accounts::Accounts,
        auth::Auth,
        catalogue::Catalogue,
        execution::Execution,
        sessions::Sessions,
        sources::{Source, Sources, Transport},
        workspaces::Workspaces,
    },
    infrastructure::{postgres::Database, sandbox::MontySandbox},
};
use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use sqlx::ConnectOptions;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone)]
struct Provider {
    name: &'static str,
    calls: Arc<AtomicUsize>,
}
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct Echo {
    value: String,
}
#[tool_router]
impl Provider {
    #[tool(description = "Echo a value", annotations(read_only_hint = true))]
    async fn echo(&self, Parameters(input): Parameters<Echo>) -> CallToolResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string(&format!("{}:{}", self.name, input.value)).unwrap(),
        )])
    }
}
#[tool_handler]
impl ServerHandler for Provider {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}
fn upstream(provider: Provider) -> StreamableHttpService<Provider, LocalSessionManager> {
    StreamableHttpService::new(
        move || Ok(provider.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true),
    )
}

pub(super) async fn rpc(
    client: &reqwest::Client,
    url: &str,
    host: &str,
    token: &str,
    version: &str,
    method: &str,
    mut params: serde_json::Value,
) -> serde_json::Value {
    let mut request = client
        .post(format!("{url}/mcp"))
        .header("host", host)
        .bearer_auth(token)
        .header("accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", version);
    if version == "2026-07-28" {
        request = request.header("Mcp-Method", method);
        if method == "tools/call" {
            request = request.header("Mcp-Name", params["name"].as_str().unwrap());
        }
        params["_meta"] = serde_json::json!({"io.modelcontextprotocol/protocolVersion":version,"io.modelcontextprotocol/clientCapabilities":{},"io.modelcontextprotocol/clientInfo":{"name":"endpoint-test","version":"1"}});
    }
    let response = request
        .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response = response.json::<serde_json::Value>().await.unwrap();
    assert!(response.get("error").is_none(), "{response}");
    response["result"].clone()
}
async fn task_rpc(
    client: &reqwest::Client,
    url: &str,
    host: &str,
    token: &str,
    method: &str,
    mut params: serde_json::Value,
    capable: bool,
) -> serde_json::Value {
    let caps = if capable {
        serde_json::json!({"extensions":{"io.modelcontextprotocol/tasks":{}}})
    } else {
        serde_json::json!({})
    };
    params["_meta"] = serde_json::json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":caps,"io.modelcontextprotocol/clientInfo":{"name":"task-test","version":"1"}});
    let mut request = client
        .post(format!("{url}/mcp"))
        .header("host", host)
        .bearer_auth(token)
        .header("accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", method);
    if method == "tools/call" {
        request = request.header("Mcp-Name", params["name"].as_str().unwrap());
    } else {
        request = request.header("Mcp-Name", params["taskId"].as_str().unwrap());
    }
    let response = request
        .json(&serde_json::json!({"jsonrpc":"2.0","id":9,"method":method,"params":params}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let response: serde_json::Value = response.json().await.unwrap();
    assert!(
        status == 200 || (status == 400 && response.get("error").is_some()),
        "{method}: {status} {response}"
    );
    response
}
async fn terminal_task(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    id: &str,
) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let response = task_rpc(
                client,
                url,
                "test.example",
                token,
                "tasks/get",
                serde_json::json!({"taskId":id}),
                true,
            )
            .await;
            assert!(response.get("error").is_none(), "{response}");
            let result = response["result"].clone();
            if result["status"] != "working" {
                return result;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

pub(super) fn tool_result(result: serde_json::Value) -> serde_json::Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[sqlx::test(migrations = "../core/migrations")]
async fn hostname_tokens_and_scopes_hold_for_both_mcp_protocol_eras(pool: sqlx::PgPool) {
    let db = Database::connect(pool.connect_options().to_url_lossy().as_str())
        .await
        .unwrap();
    let endpoints = Workspaces::new(db.clone(), "test.example".into()).unwrap();
    endpoints.initialize().await.unwrap();
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
    let catalogue = Catalogue::new(db.clone());
    let sources = Sources::new(
        db.clone(),
        catalogue.clone(),
        accounts.clone(),
        endpoints.clone(),
    );
    let alpha = Arc::new(AtomicUsize::new(0));
    let beta = Arc::new(AtomicUsize::new(0));
    let upstream_app = Router::new()
        .nest_service(
            "/alpha/mcp",
            upstream(Provider {
                name: "alpha",
                calls: alpha.clone(),
            }),
        )
        .nest_service(
            "/beta/mcp",
            upstream(Provider {
                name: "beta",
                calls: beta.clone(),
            }),
        );
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream_listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let tasks = TaskTracker::new();
    let upstream_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(upstream_listener, upstream_app)
            .with_graceful_shutdown(upstream_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let mut ids = Vec::new();
    for name in ["alpha", "beta"] {
        ids.push(
            sources
                .save(
                    None,
                    Source {
                        slug: name.into(),
                        requires_account: false,
                        transport: Transport::McpHttp {
                            url: format!("http://{upstream_address}/{name}/mcp"),
                            bearer_env: None,
                        },
                    },
                )
                .await
                .unwrap(),
        );
    }
    let work = endpoints.create("work", &[ids[0]]).await.unwrap();
    let home = endpoints.create("home", &[ids[1]]).await.unwrap();
    let source_task = sources.clone();
    let source_stop = stop.clone();
    tasks.spawn(async move {
        source_task.run(source_stop).await;
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while sources.lookup("alpha.echo").await.is_none()
            || sources.lookup("beta.echo").await.is_none()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both upstreams should synchronize");
    let auth = Auth::new(db.clone());
    let (work_token_id, work_token) = auth
        .create_for_workspace("work", work, "test.example", "/mcp")
        .await
        .unwrap()
        .unwrap();
    let (_, home_token) = auth
        .create_for_workspace("home", home, "test.example", "/mcp")
        .await
        .unwrap()
        .unwrap();
    let (_, main_token) = auth.create("main", "test.example").await.unwrap();
    let worker = std::env::var_os("CODEMODE_MONTY_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.tools/monty/bin/monty")
        });
    let sandbox = Arc::new(
        MontySandbox::new(&worker, Duration::from_secs(1), 32768)
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
    let state = server::State {
        mcp_tasks: server::Tasks::default(),
        rate_limit: Arc::new(
            codemode_core::components::auth::rate_limit::RateLimiter::new(60, 600),
        ),
        oauth: Arc::new(codemode_core::components::auth::oauth::OAuth::new(
            db.clone(),
            "test.example".into(),
        )),
        db: db.clone(),
        auth,
        catalogue,
        execution,
        hostname: "test.example".into(),
        sessions: Arc::new(Sessions::new(db)),
        accounts: Arc::new(accounts),
        sources: Arc::new(sources),
        workspaces: Arc::new(endpoints.clone()),
        stop: stop.clone(),
    };
    let task_state = state.clone();
    let limited_auth = state.auth.clone();
    let limited_state = server::State {
        rate_limit: Arc::new(codemode_core::components::auth::rate_limit::RateLimiter::new(1, 2)),
        ..state.clone()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(listener, server::router(state))
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let url = format!("http://{address}");
    let client = reqwest::Client::new();
    for host in ["test.example"] {
        for path in [
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-protected-resource/mcp",
        ] {
            let response = client
                .get(format!("{url}{path}"))
                .header("host", host)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let metadata = response.json::<serde_json::Value>().await.unwrap();
            assert_eq!(metadata["resource"], format!("https://{host}/mcp"));
            assert_eq!(
                metadata["authorization_servers"],
                serde_json::json!(["https://test.example"])
            );
        }
        let response = client
            .post(format!("{url}/mcp"))
            .header("host", host)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
        assert_eq!(
            response.headers()["www-authenticate"],
            format!(
                "Bearer resource_metadata=\"https://{host}/.well-known/oauth-protected-resource/mcp\", scope=\"mcp\""
            )
        );
    }
    let response = client
        .get(format!("{url}/.well-known/oauth-protected-resource/mcp"))
        .header("host", "unknown.endpoints.example")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    for (host, token, status) in [
        ("work.endpoints.example", work_token.as_str(), 404),
        ("home.endpoints.example", home_token.as_str(), 404),
        ("unknown.endpoints.example", work_token.as_str(), 404),
        ("test.example:444", work_token.as_str(), 404),
    ] {
        let response = client
            .post(format!("{url}/mcp"))
            .header("host", host)
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    for version in ["2025-11-25", "2026-07-28"] {
        for (host, token) in [
            ("test.example", work_token.as_str()),
            ("test.example", home_token.as_str()),
            ("test.example", main_token.as_str()),
        ] {
            if version == "2025-11-25" {
                rpc(&client,&url,host,token,version,"initialize",serde_json::json!({"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"endpoint-test","version":"1"}})).await;
            }
            let listed = rpc(
                &client,
                &url,
                host,
                token,
                version,
                "tools/list",
                serde_json::json!({}),
            )
            .await;
            assert_eq!(listed["tools"].as_array().unwrap().len(), 2);
            assert_eq!(
                listed["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| t["name"].as_str().unwrap())
                    .collect::<std::collections::BTreeSet<_>>(),
                std::collections::BTreeSet::from(["search", "execute"])
            );
        }
        for (host, token, expected) in [
            ("test.example", work_token.as_str(), "alpha.echo"),
            ("test.example", home_token.as_str(), "beta.echo"),
        ] {
            let page = tool_result(
                rpc(
                    &client,
                    &url,
                    host,
                    token,
                    version,
                    "tools/call",
                    serde_json::json!({"name":"search","arguments":{"query":""}}),
                )
                .await,
            );
            assert_eq!(page["tools"].as_array().unwrap().len(), 1);
            assert_eq!(page["tools"][0]["path"], expected);
        }
        let page = tool_result(
            rpc(
                &client,
                &url,
                "test.example",
                &main_token,
                version,
                "tools/call",
                serde_json::json!({"name":"search","arguments":{"query":""}}),
            )
            .await,
        );
        assert_eq!(page["tools"].as_array().unwrap().len(), 2);
        let result=tool_result(rpc(&client,&url,"TEST.example:443",&work_token,version,"tools/call",serde_json::json!({"name":"execute","arguments":{"code":"return await tools.alpha.echo(value='ok')"}})).await);
        assert_eq!(result["status"], "completed");
        assert_eq!(result["result"], "alpha:ok");
        let before = beta.load(Ordering::SeqCst);
        let hidden=tool_result(rpc(&client,&url,"test.example",&work_token,version,"tools/call",serde_json::json!({"name":"execute","arguments":{"code":"return await tools.beta.echo(value='hidden')"}})).await);
        assert_eq!(hidden["status"], "failed");
        let result=tool_result(rpc(&client,&url,"test.example",&work_token,version,"tools/call",serde_json::json!({"name":"execute","arguments":{"code":"return await _call('beta.echo', '{\"value\":\"forbidden\"}')"}})).await);
        assert_eq!(result["status"], "failed");
        assert_eq!(beta.load(Ordering::SeqCst), before);
    }
    let created = task_rpc(
        &client,
        &url,
        "test.example",
        &work_token,
        "tools/call",
        serde_json::json!({"name":"execute","arguments":{"code":"return 42"}}),
        true,
    )
    .await;
    assert_eq!(created["result"]["resultType"], "task");
    let id = created["result"]["taskId"].as_str().unwrap();
    let done = terminal_task(&client, &url, &work_token, id).await;
    assert_eq!(done["status"], "completed");
    assert_eq!(tool_result(done["result"].clone())["result"], 42);
    let (_, other_work_token) = limited_auth
        .create_for_workspace("another caller", work, "test.example", "/mcp")
        .await
        .unwrap()
        .unwrap();
    for (host, token) in [
        ("test.example", other_work_token.as_str()),
        ("test.example", home_token.as_str()),
    ] {
        for method in ["tasks/get", "tasks/update", "tasks/cancel"] {
            let params = if method == "tasks/update" {
                serde_json::json!({"taskId":id,"inputResponses":{"approve":true}})
            } else {
                serde_json::json!({"taskId":id})
            };
            let response = task_rpc(&client, &url, host, token, method, params, true).await;
            assert_eq!(response["error"]["code"], -32602, "{response}");
        }
    }
    let unsupported = task_rpc(
        &client,
        &url,
        "test.example",
        &work_token,
        "tasks/get",
        serde_json::json!({"taskId":id}),
        false,
    )
    .await;
    assert!(unsupported.get("error").is_some());
    let created = task_rpc(
        &client,
        &url,
        "test.example",
        &work_token,
        "tools/call",
        serde_json::json!({"name":"execute","arguments":{"code":"print('before cancellation')\nawait tools.alpha.echo(value='cancellation marker')\nwhile True: pass"}}),
        true,
    )
    .await;
    let running_id = created["result"]["taskId"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(1),async {
        loop {
            let running:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM executions e WHERE token_id=$1 AND code LIKE '%while True: pass%' AND status='running' AND EXISTS(SELECT 1 FROM tool_calls c WHERE c.execution_id=e.id AND c.outcome='ok'))").bind(work_token_id).fetch_one(&pool).await.unwrap();
            if running { break; } tokio::task::yield_now().await;
        }
    }).await.unwrap();
    let cancelled = task_rpc(
        &client,
        &url,
        "test.example",
        &work_token,
        "tasks/cancel",
        serde_json::json!({"taskId":running_id}),
        true,
    )
    .await;
    assert!(cancelled.get("error").is_none());
    assert_eq!(
        terminal_task(&client, &url, &work_token, running_id).await["status"],
        "cancelled"
    );
    tokio::time::timeout(Duration::from_secs(1),async {
        loop {
            let finished:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM executions WHERE token_id=$1 AND code LIKE '%while True: pass%' AND status='failed' AND error LIKE '%cancelled%')").bind(work_token_id).fetch_one(&pool).await.unwrap();
            if finished { break; } tokio::task::yield_now().await;
        }
    }).await.unwrap();
    let output:String=sqlx::query_scalar("SELECT output FROM executions WHERE token_id=$1 AND code LIKE '%while True: pass%' ORDER BY started_at DESC LIMIT 1").bind(work_token_id).fetch_one(&pool).await.unwrap();
    assert_eq!(output, "before cancellation\n");
    let revision = task_state
        .sources
        .get(ids[0])
        .await
        .unwrap()
        .unwrap()
        .revision;
    task_state
        .sources
        .policies()
        .set(
            work,
            ids[0],
            revision,
            "echo",
            codemode_core::components::policies::Choice::Block,
        )
        .await
        .unwrap();
    let before = alpha.load(Ordering::SeqCst);
    let created=task_rpc(&client,&url,"test.example",&work_token,"tools/call",serde_json::json!({"name":"execute","arguments":{"code":"try:\n    return await tools.alpha.echo(value='task policy')\nexcept PermissionError as e:\n    return str(e)"}}),true).await;
    let task_id = created["result"]["taskId"].as_str().unwrap();
    let blocked =
        tool_result(terminal_task(&client, &url, &work_token, task_id).await["result"].clone());
    assert_eq!(blocked["status"], "completed");
    assert!(blocked["result"].as_str().unwrap().contains("blocked"));
    assert!(
        blocked["result"]
            .as_str()
            .unwrap()
            .contains(&work.to_string())
    );
    assert_eq!(alpha.load(Ordering::SeqCst), before);
    let updated = task_rpc(
        &client,
        &url,
        "test.example",
        &work_token,
        "tasks/update",
        serde_json::json!({"taskId":task_id,"inputResponses":{"approve":true,"allow":true}}),
        true,
    )
    .await;
    assert!(updated.get("error").is_none());
    assert_eq!(alpha.load(Ordering::SeqCst), before);
    let removed = task_rpc(
        &client,
        &url,
        "test.example",
        &work_token,
        "tools/call",
        serde_json::json!({"name":"resume","arguments":{"executionId":blocked["executionId"]}}),
        false,
    )
    .await;
    assert!(removed.get("error").is_some() || removed["isError"] == true);
    task_state
        .sources
        .policies()
        .set(
            work,
            ids[0],
            revision,
            "echo",
            codemode_core::components::policies::Choice::Default,
        )
        .await
        .unwrap();
    task_state.mcp_tasks.shutdown();
    let disappeared = task_rpc(
        &client,
        &url,
        "test.example",
        &work_token,
        "tasks/get",
        serde_json::json!({"taskId":id}),
        true,
    )
    .await;
    assert!(disappeared.get("error").is_some());
    endpoints.update(work, "work", &[]).await.unwrap();
    let page = tool_result(
        rpc(
            &client,
            &url,
            "test.example",
            &work_token,
            "2026-07-28",
            "tools/call",
            serde_json::json!({"name":"search","arguments":{"query":""}}),
        )
        .await,
    );
    assert!(page["tools"].as_array().unwrap().is_empty());
    let before = alpha.load(Ordering::SeqCst);
    let result=tool_result(rpc(&client,&url,"test.example",&work_token,"2026-07-28","tools/call",serde_json::json!({"name":"execute","arguments":{"code":"return await _call('alpha.echo', '{\"value\":\"removed\"}')"}})).await);
    assert_eq!(result["status"], "failed");
    assert_eq!(alpha.load(Ordering::SeqCst), before);
    let blocked:i64=sqlx::query_scalar("SELECT count(*) FROM tool_calls c JOIN executions e ON e.id=c.execution_id WHERE e.token_id=$1 AND c.outcome='blocked'").bind(work_token_id).fetch_one(&pool).await.unwrap();
    assert_eq!(blocked, 4);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let limited_url = format!("http://{}", listener.local_addr().unwrap());
    let limited_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(listener, server::router(limited_state))
            .with_graceful_shutdown(limited_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let (_, second) = limited_auth
        .create("second rate-limit caller", "test.example")
        .await
        .unwrap();
    let (_, third) = limited_auth
        .create("third rate-limit caller", "test.example")
        .await
        .unwrap();
    for (token, status) in [
        (&main_token, 200),
        (&main_token, 429),
        (&second, 200),
        (&third, 429),
    ] {
        let response = client
            .post(format!("{limited_url}/mcp"))
            .header("host", "test.example")
            .bearer_auth(token)
            .header("accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        if status == 429 {
            assert!(
                (1..=60).contains(
                    &response.headers()["retry-after"]
                        .to_str()
                        .unwrap()
                        .parse::<u64>()
                        .unwrap()
                )
            );
        }
    }
    stop.cancel();
    tasks.close();
    tasks.wait().await;
    sandbox.pool.close().await;
}
