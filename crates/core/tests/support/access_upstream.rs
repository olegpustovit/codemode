use axum::{
    Json, Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Default)]
pub struct Counters {
    pub calls: Arc<AtomicUsize>,
    pub streams: Arc<AtomicUsize>,
    pub closed: Arc<AtomicUsize>,
    pub deleted: Arc<AtomicUsize>,
}
pub struct Upstream {
    pub url: String,
    pub counters: Counters,
    server: tokio::task::JoinHandle<()>,
}
impl Upstream {
    pub async fn new() -> Self {
        let counters = Counters::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/mcp", any(request))
            .with_state(counters.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            url,
            counters,
            server,
        }
    }
}
impl Drop for Upstream {
    fn drop(&mut self) {
        self.server.abort();
    }
}
struct Open(Counters);
impl Drop for Open {
    fn drop(&mut self) {
        self.0.streams.fetch_sub(1, Ordering::SeqCst);
        self.0.closed.fetch_add(1, Ordering::SeqCst);
    }
}
fn pending(counters: Counters) -> Response {
    counters.streams.fetch_add(1, Ordering::SeqCst);
    let open = Open(counters);
    let stream = futures_util::stream::once(async move {
        let _open = open;
        std::future::pending::<Result<&'static [u8], std::io::Error>>().await
    });
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
        .into_response()
}
async fn request(
    State(counters): State<Counters>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    assert_eq!(
        headers.get("authorization").unwrap(),
        "Bearer eval-provider-token"
    );
    match *request.method() {
        axum::http::Method::GET => return pending(counters),
        axum::http::Method::DELETE => {
            counters.deleted.fetch_add(1, Ordering::SeqCst);
            return StatusCode::NO_CONTENT.into_response();
        }
        axum::http::Method::POST => {}
        _ => return StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
    let bytes = axum::body::to_bytes(request.into_body(), 65536)
        .await
        .unwrap();
    let rpc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let result = match rpc["method"].as_str().unwrap() {
        "initialize" => {
            serde_json::json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"eval-access-upstream","version":"1"}})
        }
        "tools/list" => {
            serde_json::json!({"tools":[{"name":"wait","description":"Owned pending upstream response","inputSchema":{"type":"object","properties":{}},"annotations":{"readOnlyHint":true}}]})
        }
        "tools/call" => {
            counters.calls.fetch_add(1, Ordering::SeqCst);
            return pending(counters);
        }
        _ if rpc.get("id").is_none() => return StatusCode::ACCEPTED.into_response(),
        "ping" => serde_json::json!({}),
        method => panic!("unexpected method: {method}"),
    };
    (
        [("mcp-session-id", "eval-session")],
        Json(serde_json::json!({"jsonrpc":"2.0","id":rpc["id"],"result":result})),
    )
        .into_response()
}
