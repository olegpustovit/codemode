use super::*;
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::post,
};
use codemode_core::{
    components::{
        accounts::{Accounts, HttpCredential},
        sources::{Source, Transport},
    },
    infrastructure::sandbox::MontySandbox,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Default)]
struct Api {
    calls: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
    redirect: Arc<Mutex<String>>,
    leaks: Arc<AtomicUsize>,
}
async fn serve(
    State(state): State<Api>,
    Query(params): Query<BTreeMap<String, String>>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    state
        .calls
        .lock()
        .unwrap()
        .push((headers.clone(), body.clone()));
    let mode = body["variables"]["mode"].as_str().unwrap_or("");
    match mode {
        "redirect" => (
            StatusCode::TEMPORARY_REDIRECT,
            [("location", state.redirect.lock().unwrap().clone())],
        )
            .into_response(),
        "large" => (
            [("content-type", "application/json")],
            "x".repeat(8 * 1024 * 1024 + 1),
        )
            .into_response(),
        "invalid" => "not JSON".into_response(),
        "badstatus" => (
            StatusCode::BAD_REQUEST,
            Json(json!({"data":{"mustNotSucceed":true}})),
        )
            .into_response(),
        "error" | "partial" => {
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let key = headers
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let message = format!(
                "fixture failure {} {} {} {} {}",
                auth,
                auth.strip_prefix("Bearer ").unwrap_or(""),
                key,
                params.get("key").map(String::as_str).unwrap_or(""),
                uri
            );
            (
                if mode == "error" {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::OK
                },
                Json(json!({"data":{"partial":true},"errors":[{"message":message}]})),
            )
                .into_response()
        }
        _ => Json(json!({"data":{"items":[{"id":"one"}],"variables":body["variables"]}}))
            .into_response(),
    }
}
async fn wait(sources: &Sources, id: Uuid) {
    sources.wake();
    let revision = sources.get(id).await.unwrap().unwrap().revision;
    let ready = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if sources
                .lookup("graph.query")
                .await
                .is_some_and(|tool| tool.revision == revision)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        ready.is_ok(),
        "source did not publish: {}",
        serde_json::to_string(&sources.get(id).await.unwrap()).unwrap()
    );
}
#[sqlx::test(migrations = "./migrations")]
async fn graphql_source_validates_operation_kind_links_credentials_and_dispatches_mutations(
    pool: sqlx::PgPool,
) {
    let stop = CancellationToken::new();
    let tasks = TaskTracker::new();
    let state = Api::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/graphql/", listener.local_addr().unwrap());
    let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    *state.redirect.lock().unwrap() = format!("http://{}/leak", other.local_addr().unwrap());
    let leaks = state.leaks.clone();
    let other_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(
            other,
            Router::new().fallback(move || {
                let leaks = leaks.clone();
                async move {
                    leaks.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        )
        .with_graceful_shutdown(other_stop.cancelled_owned())
        .await
        .unwrap();
    });
    let server_stop = stop.clone();
    let router = Router::new()
        .route("/graphql/", post(serve))
        .with_state(state.clone());
    tasks.spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let db = database(&pool).await;
    codemode_core::components::workspaces::Workspaces::new(db.clone(), "test.example".into())
        .unwrap()
        .initialize()
        .await
        .unwrap();
    let catalogue = Catalogue::new(db.clone());
    let sources = managed_sources(db.clone(), catalogue.clone());
    let id = sources
        .save(
            None,
            Source {
                slug: "graph".into(),
                requires_account: true,
                transport: Transport::Graphql { url: url.clone() },
            },
        )
        .await
        .unwrap();
    let sync = sources.clone();
    let sync_stop = stop.clone();
    tasks.spawn(async move {
        sync.run(sync_stop).await;
    });
    sources.ready().await;
    assert_eq!(sources.get(id).await.unwrap().unwrap().status, "needs_link");
    let auth = Auth::new(db.clone());
    let (_, token) = auth.create("test", "test.example").await.unwrap();
    let principal = auth
        .authenticate(&token, "test.example")
        .await
        .unwrap()
        .unwrap();
    let sandbox = Arc::new(
        MontySandbox::new(&super::policies::binary(), Duration::from_secs(1), 32768)
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
    let run=execution.execute(principal.clone(),"try:\n    return await _call('graph.query', '{}')\nexcept ToolError as e:\n    return str(e)".into()).await.unwrap();
    assert!(
        run.result
            .get()
            .contains(&format!("/sources/{id}/accounts"))
    );
    assert!(state.calls.lock().unwrap().is_empty());
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
    accounts
        .link_http(
            id,
            HttpCredential::ApiKeyHeader {
                name: "x-api-key".into(),
                credential: "host-side-secret".into(),
            },
        )
        .await
        .unwrap();
    sources.wake();
    wait(&sources, id).await;
    assert_eq!(
        catalogue
            .search("GraphQL", true, None, None)
            .await
            .unwrap()
            .tools
            .len(),
        2
    );
    let run=execution.execute(principal.clone(),"return await tools.graph.query(query='query Read($id:ID!) { item(id:$id) { ...Item } } mutation Other { deleteItem } fragment Item on Thing { id }', operationName='Read', variables={'id':'one'})".into()).await.unwrap();
    assert_eq!(run.status, "completed");
    assert_eq!(
        serde_json::from_str::<Value>(run.result.get()).unwrap()["items"][0]["id"],
        "one"
    );
    {
        let calls = state.calls.lock().unwrap();
        let (headers, body) = calls.last().unwrap();
        assert_eq!(headers["x-api-key"], "host-side-secret");
        assert!(headers.get("authorization").is_none());
        assert_eq!(body["variables"]["id"], "one");
        assert_eq!(body["operationName"], "Read");
        assert!(body["query"].as_str().unwrap().contains("fragment Item"));
        assert!(!body["query"].as_str().unwrap().contains("mutation Other"));
    }
    let before = state.calls.lock().unwrap().len();
    for query in [
        "mutation { deleteItem }",
        "subscription { updates }",
        "query A { x } query B { y }",
    ] {
        let code = format!(
            "try:\n    return await tools.graph.query(query={})\nexcept ToolError as e:\n    return str(e)",
            serde_json::to_string(query).unwrap()
        );
        let run = execution.execute(principal.clone(), code).await.unwrap();
        assert_eq!(run.status, "completed");
        assert!(!run.result.get().contains("items"));
    }
    assert_eq!(state.calls.lock().unwrap().len(), before);
    let mutation=execution.execute(principal.clone(),"return await tools.graph.mutation(query='mutation Delete($id:ID!) { deleteItem(id:$id) }',variables={'id':'one'})".into()).await.unwrap();
    assert_eq!(mutation.status, "completed");
    assert_eq!(state.calls.lock().unwrap().len(), before + 1);
    for mode in [
        "error",
        "partial",
        "redirect",
        "invalid",
        "badstatus",
        "large",
    ] {
        let code = format!(
            "try:\n    await tools.graph.query(query='{{ viewer {{ id }} }}', variables={{'mode':'{mode}'}})\n    return 'unexpected success'\nexcept ToolError as e:\n    return str(e)"
        );
        let run = execution.execute(principal.clone(), code).await.unwrap();
        assert_eq!(run.status, "completed", "{mode}: {:?}", run.error);
        assert!(!run.result.get().contains("unexpected success"), "{mode}");
        assert!(!run.result.get().contains("host-side-secret"));
        if mode == "error" || mode == "partial" {
            assert!(run.result.get().contains("fixture failure"));
            assert!(run.result.get().contains("REDACTED"));
        }
    }
    for credential in [
        HttpCredential::Bearer {
            credential: "linked-bearer-secret".into(),
        },
        HttpCredential::ApiKeyQuery {
            name: "key".into(),
            credential: "linked &query=secret/é".into(),
        },
        HttpCredential::Basic {
            username: "host-user".into(),
            password: "host-password".into(),
        },
    ] {
        accounts.link_http(id, credential).await.unwrap();
        wait(&sources, id).await;
        let run=execution.execute(principal.clone(),"try:\n    return await tools.graph.query(query='{ x }', variables={'mode':'error'})\nexcept ToolError as e:\n    return str(e)".into()).await.unwrap();
        assert!(run.result.get().contains("fixture failure"));
        for secret in [
            "linked-bearer-secret",
            "linked &query=secret/é",
            "linked+%26query%3Dsecret%2F%C3%A9",
            "host-password",
            "aG9zdC11c2VyOmhvc3QtcGFzc3dvcmQ=",
        ] {
            assert!(!run.result.get().contains(secret));
        }
    }
    assert_eq!(state.leaks.load(Ordering::SeqCst), 0);
    let audit: Vec<String> = sqlx::query_scalar("SELECT COALESCE(error,'') FROM tool_calls")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(audit.iter().all(|error| !error.contains("host-side-secret")
        && !error.contains("linked-bearer-secret")
        && !error.contains("linked &query=secret/é")
        && !error.contains("linked+%26query%3Dsecret%2F%C3%A9")));
    assert!(
        !serde_json::to_string(&sources.list().await.unwrap())
            .unwrap()
            .contains("host-side-secret")
    );
    sources
        .save(
            Some(id),
            Source {
                slug: "graph".into(),
                requires_account: true,
                transport: Transport::Graphql {
                    url: format!("{url}changed"),
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
    db.close().await;
}
