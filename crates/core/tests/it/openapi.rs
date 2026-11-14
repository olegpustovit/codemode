use super::*;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, put},
};
use codemode_core::components::{
    accounts::{Accounts, HttpCredential},
    sources::{Source, Transport},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
#[derive(Clone)]
struct Api {
    server: Arc<Mutex<String>>,
    calls: Arc<Mutex<Vec<(HeaderMap, BTreeMap<String, String>, Value)>>>,
    leaks: Arc<AtomicUsize>,
}
use serde_json::Value;
async fn spec(State(state): State<Api>) -> Json<Value> {
    Json(
        json!({"openapi":"3.1.0","servers":[{"url":state.server.lock().unwrap().clone()}],"paths":{
        "/things/{id}":{"put":{"operationId":"putThing","parameters":[
            {"name":"id","in":"path","required":true,"schema":{"type":"string"}},
            {"name":"X-Trace","in":"header","schema":{"type":"string"}},
            {"name":"filter","in":"query","schema":{"type":"string"}}
        ],"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Body"}}}},"responses":{"200":{"content":{"application/json":{"schema":{"type":"object"}}}}}}},
        "/fail":{"get":{"operationId":"fail","responses":{}}},
        "/redirect":{"get":{"operationId":"redirect","responses":{}}}
    },"components":{"schemas":{"Body":{"type":"object","properties":{"value":{"type":"string"},"password":{"type":"string"}},"required":["value"]}}}}),
    )
}
async fn update(
    State(state): State<Api>,
    Path(id): Path<String>,
    Query(query): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    state
        .calls
        .lock()
        .unwrap()
        .push((headers, query, body.clone()));
    Json(json!({"id":id,"value":body["value"]}))
}
async fn wait(sources: &Sources, id: Uuid) {
    let revision = sources.get(id).await.unwrap().unwrap().revision;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if sources
                .lookup("api.putThing")
                .await
                .is_some_and(|tool| tool.revision == revision)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[sqlx::test(migrations = "./migrations")]
async fn openapi_source_pins_server_dispatches_encrypted_credentials_and_never_follows_redirects(
    pool: sqlx::PgPool,
) {
    let stop = CancellationToken::new();
    let tasks = TaskTracker::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let state = Api {
        server: Arc::new(Mutex::new(format!("http://{address}/v1"))),
        calls: Default::default(),
        leaks: Default::default(),
    };
    let leak = state.leaks.clone();
    let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let other_url = format!("http://{}", other.local_addr().unwrap());
    let other_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(
            other,
            Router::new().fallback(move || {
                let leak = leak.clone();
                async move {
                    leak.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        )
        .with_graceful_shutdown(other_stop.cancelled_owned())
        .await
        .unwrap();
    });
    let redirect = other_url.clone();
    let router = Router::new()
        .route("/spec", get(spec))
        .route("/v1/things/{id}", put(update))
        .route(
            "/v1/fail",
            get(|| async {
                (
                    StatusCode::UNAUTHORIZED,
                    "fixture-upstream-secret must not reach the model",
                )
            }),
        )
        .route(
            "/v1/redirect",
            get(move || {
                let url = redirect.clone();
                async move { (StatusCode::FOUND, [("location", url)]) }
            }),
        )
        .with_state(state.clone());
    let server_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let db = database(&pool).await;
    let catalogue = Catalogue::new(db.clone());
    let sources = managed_sources(db.clone(), catalogue.clone());
    let id = sources
        .save(
            None,
            Source {
                slug: "api".into(),
                requires_account: true,
                transport: Transport::OpenApi {
                    url: format!("http://{address}/spec"),
                    server_url: None,
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(sources.get(id).await.unwrap().unwrap()).unwrap()["serverUrl"],
        format!("http://{address}/v1")
    );
    let sync = sources.clone();
    let sync_stop = stop.clone();
    tasks.spawn(async move {
        sync.run(sync_stop).await;
    });
    wait(&sources, id).await;
    assert_eq!(
        catalogue
            .search("thing", false, None, None)
            .await
            .unwrap()
            .tools
            .len(),
        1
    );
    let auth = Auth::new(db.clone());
    let (_, token) = auth.create("test", "test.example").await.unwrap();
    let principal = auth
        .authenticate(&token, "test.example")
        .await
        .unwrap()
        .unwrap();
    let binary = std::env::var_os("CODEMODE_MONTY_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.tools/monty/bin/monty")
        });
    let sandbox = Arc::new(
        codemode_core::infrastructure::sandbox::MontySandbox::new(
            &binary,
            Duration::from_secs(1),
            [73; 32],
            32768,
        )
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
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
    let unlinked = execution
        .execute(
            principal.clone(),
            "return await tools.api.putThing(id='x',body={'value':'x'})".into(),
        )
        .await
        .unwrap();
    assert_eq!(unlinked.status, "failed");
    assert!(
        unlinked
            .error
            .as_deref()
            .unwrap()
            .contains("link it at https://test.example/sources/")
    );
    assert!(state.calls.lock().unwrap().is_empty());
    for (credential, header, query) in [
        (
            HttpCredential::Bearer {
                credential: "fixture-upstream-secret".into(),
            },
            Some(("authorization", "Bearer fixture-upstream-secret")),
            None,
        ),
        (
            HttpCredential::ApiKeyHeader {
                name: "X-Api-Key".into(),
                credential: "fixture-upstream-secret".into(),
            },
            Some(("x-api-key", "fixture-upstream-secret")),
            None,
        ),
        (
            HttpCredential::ApiKeyQuery {
                name: "api_key".into(),
                credential: "fixture-upstream-secret".into(),
            },
            None,
            Some(("api_key", "fixture-upstream-secret")),
        ),
        (
            HttpCredential::Basic {
                username: "user".into(),
                password: "password".into(),
            },
            Some(("authorization", "Basic dXNlcjpwYXNzd29yZA==")),
            None,
        ),
    ] {
        accounts.link_http(id, credential).await.unwrap();
        sources.wake();
        wait(&sources, id).await;
        let result=execution.execute(principal.clone(),"return await tools.api.putThing(id='a/b',filter='x&y',X_Trace='trace',body={'value':'ok','password':'agent-input-secret'})".into()).await.unwrap();
        assert_eq!(result.status, "completed", "{:?}", result.error);
        assert_eq!(
            serde_json::from_str::<Value>(result.result.get()).unwrap(),
            json!({"id":"a/b","value":"ok"})
        );
        {
            let calls = state.calls.lock().unwrap();
            let (headers, query_values, body) = calls.last().unwrap();
            assert_eq!(headers["x-trace"], "trace");
            assert_eq!(query_values["filter"], "x&y");
            assert_eq!(body["value"], "ok");
            if let Some((name, value)) = header {
                assert_eq!(headers[name], value);
            }
            if let Some((name, value)) = query {
                assert_eq!(query_values[name], value);
                assert!(!headers.contains_key("authorization"));
            }
            assert!(
                headers
                    .get("authorization")
                    .is_none_or(|v| v.as_bytes() != token.as_bytes())
            );
        }
        let audit: String =
            sqlx::query_scalar("SELECT args_redacted::text FROM tool_calls WHERE execution_id=$1")
                .bind(result.execution_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            audit.contains("[REDACTED]")
                && !audit.contains("agent-input-secret")
                && !audit.contains("fixture-upstream-secret")
        );
    }
    let count = state.calls.lock().unwrap().len();
    let invalid = execution
        .execute(
            principal.clone(),
            "return await tools.api.putThing(id=123,body={'value':'ok'})".into(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status, "failed");
    assert_eq!(state.calls.lock().unwrap().len(), count);
    for (tool, status) in [("fail", 401), ("redirect", 302)] {
        let result = execution
            .execute(
                principal.clone(),
                format!(
                    "try:\n    await tools.api.{tool}()\nexcept ToolError as e:\n    return str(e)"
                ),
            )
            .await
            .unwrap();
        assert_eq!(result.status, "completed");
        assert!(result.result.get().contains(&format!("HTTP {status}")));
        assert!(!result.result.get().contains("fixture-upstream-secret"));
    }
    assert_eq!(state.leaks.load(Ordering::SeqCst), 0);
    *state.server.lock().unwrap() = other_url.clone();
    sources.resync(id).await.unwrap();
    wait(&sources, id).await;
    assert_eq!(
        sources.get(id).await.unwrap().unwrap().source.transport,
        Transport::OpenApi {
            url: format!("http://{address}/spec"),
            server_url: Some(format!("http://{address}/v1"))
        }
    );
    let result = execution
        .execute(
            principal.clone(),
            "return await tools.api.putThing(id='pinned',body={'value':'ok'})".into(),
        )
        .await
        .unwrap();
    assert_eq!(result.status, "completed");
    assert_eq!(state.leaks.load(Ordering::SeqCst), 0);
    let metadata = serde_json::to_string(&accounts.list().await.unwrap()).unwrap();
    assert!(
        !metadata.contains("password")
            && !metadata.contains("ciphertext")
            && !metadata.contains("fixture-upstream-secret")
    );
    sources
        .save(
            Some(id),
            Source {
                slug: "api".into(),
                requires_account: true,
                transport: Transport::OpenApi {
                    url: format!("http://{address}/spec"),
                    server_url: Some(other_url),
                },
            },
        )
        .await
        .unwrap();
    assert!(accounts.list().await.unwrap().is_empty());
    stop.cancel();
    tasks.close();
    tasks.wait().await;
    sandbox.pool.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_document_imports_fail_fast_at_the_shared_limit(pool: sqlx::PgPool) {
    let db = database(&pool).await;
    let sources = managed_sources(db.clone(), Catalogue::new(db));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/spec", listener.local_addr().unwrap());
    let (entered, mut arrivals) = tokio::sync::mpsc::channel(4);
    let (release, gate) = tokio::sync::watch::channel(false);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let router = Router::new().route(
        "/spec",
        get(move || {
            let entered = entered.clone();
            let mut gate = gate.clone();
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                entered.send(()).await.unwrap();
                while !*gate.borrow_and_update() {
                    gate.changed().await.unwrap();
                }
                Json(json!({"openapi":"3.1.0","paths":{}}))
            }
        }),
    );
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(stopped.cancelled_owned())
            .await
            .unwrap();
    });
    let mut imports = tokio::task::JoinSet::new();
    for i in 0..4 {
        let sources = sources.clone();
        let url = url.clone();
        imports.spawn(async move {
            sources
                .save(
                    None,
                    Source {
                        slug: format!("import-{i}"),
                        requires_account: false,
                        transport: Transport::OpenApi {
                            url,
                            server_url: None,
                        },
                    },
                )
                .await
        });
    }
    for _ in 0..4 {
        tokio::time::timeout(Duration::from_secs(1), arrivals.recv())
            .await
            .unwrap()
            .unwrap();
    }
    let rejected = sources
        .save(
            None,
            Source {
                slug: "overflow".into(),
                requires_account: false,
                transport: Transport::OpenApi {
                    url,
                    server_url: None,
                },
            },
        )
        .await;
    assert!(matches!(
        rejected,
        Err(codemode_core::components::sources::Error::Busy)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    release.send(true).unwrap();
    while let Some(result) = imports.join_next().await {
        result.unwrap().unwrap();
    }
    assert_eq!(sources.list().await.unwrap().len(), 4);
    stop.cancel();
    server.await.unwrap();
}
