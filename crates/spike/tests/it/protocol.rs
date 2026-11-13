use super::*;
use rmcp::{
    ClientLifecycleMode, ClientServiceExt,
    model::{CallToolRequestParams, ClientConfig, ProtocolVersion},
    transport::StreamableHttpClientTransport,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[tokio::test]
async fn one_endpoint_serves_both_protocol_eras_without_sessions() {
    let stop = CancellationToken::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let tracker = TaskTracker::new();
    let shutdown = stop.clone();
    let router = codemode_spike::server::router(stop.clone());
    tracker.spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            .unwrap();
    });
    let start = Instant::now();
    for (version, lifecycle) in [
        (
            ProtocolVersion::V_2026_07_28,
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        ),
        (
            ProtocolVersion::V_2025_11_25,
            ClientLifecycleMode::Initialize,
        ),
    ] {
        let client = ClientConfig::default()
            .with_protocol_version(version.clone())
            .serve_with_lifecycle(
                StreamableHttpClientTransport::from_uri(format!("http://{address}/mcp")),
                lifecycle,
            )
            .await
            .unwrap();
        let tools = client.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        let result = client
            .call_tool(
                CallToolRequestParams::new("echo")
                    .with_arguments(json!({"text":"both eras"}).as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        let result = serde_json::to_value(result).unwrap();
        assert_eq!(result["content"][0]["text"], "both eras");
        client.cancel().await.unwrap();
    }
    record(
        "protocol",
        json!({"versions":["2026-07-28","2025-11-25"],"stateless":true,"tool_calls":2,"rmcp":"3.10.0","elapsed_ms":start.elapsed().as_secs_f64()*1000.0}),
    );
    stop.cancel();
    tracker.close();
    tracker.wait().await;
}
