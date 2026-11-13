use std::sync::Arc;

use axum::{Router, routing::get};
use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
pub struct Echo;

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EchoArgs {
    pub text: String,
}

#[tool_router]
impl Echo {
    #[tool(
        description = "Phase 0 protocol probe: echo text",
        annotations(read_only_hint = true)
    )]
    async fn echo(&self, Parameters(args): Parameters<EchoArgs>) -> CallToolResult {
        CallToolResult::success(vec![ContentBlock::text(args.text)])
    }
}

#[tool_handler]
impl ServerHandler for Echo {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions("Phase 0 spike only; no connected tools or credentials.")
    }
}

pub fn router(stop: CancellationToken) -> Router {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_cancellation_token(stop)
        .with_allowed_hosts(["localhost", "127.0.0.1", "codemode.microapps.space"])
        .with_allowed_origins(["https://codemode.microapps.space:443"]);
    let service = StreamableHttpService::new(
        || Ok(Echo),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    Router::new()
        .route("/healthz", get(|| async { "codemode phase-0\n" }))
        .nest_service("/mcp", service)
        .layer(tower_http::limit::RequestBodyLimitLayer::new(128 * 1024))
}
