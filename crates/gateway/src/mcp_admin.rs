use crate::server::{self, State};
use axum::{
    Router,
    extract::{Request, State as AxumState},
    middleware,
    response::{IntoResponse, Response},
    routing::any,
};
use codemode_core::components::{
    auth::Principal,
    execution::log::Log,
    sources::{Source, Transport},
};
use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars,
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpService, session::local::LocalSessionManager,
    },
};
use std::{future::Future, sync::Arc};
use uuid::Uuid;

pub fn routes(state: State) -> Router<State> {
    Router::new()
        .route("/mcp-admin", any(request))
        .route("/mcp-admin/", any(request))
        .route_layer(middleware::from_fn_with_state(state, server::authenticate))
}
async fn request(AxumState(state): AxumState<State>, request: Request) -> Response {
    let config = server::transport(&state);
    let service = StreamableHttpService::new(
        move || {
            Ok(Administration {
                log: Log::new(state.db.clone()),
                state: state.clone(),
            })
        },
        Arc::new(LocalSessionManager::default()),
        config,
    );
    service.handle(request).await.into_response()
}
#[derive(Clone)]
struct Administration {
    state: State,
    log: Log,
}
fn failure(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}
impl Administration {
    async fn audited<T: serde::Serialize, E: std::error::Error>(
        &self,
        context: &RequestContext<RoleServer>,
        name: &str,
        args: serde_json::Value,
        operation: impl Future<Output = Result<T, E>>,
    ) -> CallToolResult {
        let principal = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<Principal>());
        let Some(principal) = principal.filter(|p| p.resource == "/mcp-admin") else {
            return failure("administration authorization required");
        };
        let id = match self
            .log
            .start_administration(principal.token_id, name, args)
            .await
        {
            Ok(id) => id,
            Err(error) => {
                tracing::error!(error=?error,"administration audit unavailable");
                return failure("audit unavailable; operation was not performed");
            }
        };
        let result = operation.await;
        if let Err(error) = self.log.finish_administration(id, result.is_ok()).await {
            tracing::error!(error=?error,"administration audit finalization failed");
            return failure(
                "operation finished but its audit could not be finalized; inspect the current state before repeating it",
            );
        }
        match result {
            Ok(result) => match serde_json::to_string(&result) {
                Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
                Err(_) => failure("could not encode administration result"),
            },
            Err(error) => failure(error.to_string()),
        }
    }
}
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Id {
    #[schemars(with = "String")]
    id: Uuid,
}
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveWorkspace {
    #[schemars(with = "Option<String>")]
    id: Option<Uuid>,
    name: String,
    #[schemars(with = "Vec<String>")]
    source_ids: Vec<Uuid>,
}
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SaveSource {
    #[schemars(with = "Option<String>")]
    id: Option<Uuid>,
    source: Source,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceSummary {
    id: Uuid,
    name: String,
    kind: &'static str,
    status: String,
    tool_count: i64,
    requires_account: bool,
}
#[derive(Debug, thiserror::Error)]
enum SourceError {
    #[error("{0}")]
    Input(&'static str),
    #[error(transparent)]
    Source(#[from] codemode_core::components::sources::Error),
}

#[tool_router]
impl Administration {
    #[tool(
        description = "List your workspaces, their source membership and tool scope mode.",
        annotations(read_only_hint = true)
    )]
    async fn list_workspaces(&self, context: RequestContext<RoleServer>) -> CallToolResult {
        self.audited(
            &context,
            "list_workspaces",
            serde_json::json!({}),
            self.state.workspaces.list(),
        )
        .await
    }
    #[tool(
        description = "Create a workspace with a name and source IDs, or update one by its ID. A workspace uses the shared /mcp URL and is selected during OAuth consent. The default All sources workspace is protected.",
        annotations(destructive_hint = true)
    )]
    async fn save_workspace(
        &self,
        Parameters(input): Parameters<SaveWorkspace>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let args =
            serde_json::json!({"id":input.id,"name":input.name,"sourceIds":input.source_ids});
        self.audited(&context, "save_workspace", args, async {
            if let Some(id) = input.id {
                self.state
                    .workspaces
                    .update(id, &input.name, &input.source_ids)
                    .await?;
                Ok(id)
            } else {
                self.state
                    .workspaces
                    .create(&input.name, &input.source_ids)
                    .await
            }
        })
        .await
    }
    #[tool(
        description = "Delete your custom workspace by ID. Its agent access is revoked; source connections are preserved. The default workspace cannot be deleted.",
        annotations(destructive_hint = true)
    )]
    async fn delete_workspace(
        &self,
        Parameters(input): Parameters<Id>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.audited(
            &context,
            "delete_workspace",
            serde_json::json!({"id":input.id}),
            self.state.workspaces.remove(input.id),
        )
        .await
    }
    #[tool(
        description = "List your connected sources with IDs, connection type, status and tool counts. Credentials and source configuration are never returned.",
        annotations(read_only_hint = true)
    )]
    async fn list_sources(&self, context: RequestContext<RoleServer>) -> CallToolResult {
        self.audited(&context, "list_sources", serde_json::json!({}), async {
            self.state.sources.list().await.map(|records| {
                records
                    .into_iter()
                    .map(|r| SourceSummary {
                        id: r.id,
                        name: r.source.slug,
                        status: r.status,
                        tool_count: r.tool_count,
                        requires_account: r.source.requires_account,
                        kind: match r.source.transport {
                            Transport::McpHttp { .. } => "mcp_http",
                            Transport::McpStdio { .. } => "mcp_stdio",
                            Transport::OpenApi { .. } => "openapi",
                            Transport::Graphql { .. } => "graphql",
                        },
                    })
                    .collect::<Vec<_>>()
            })
        })
        .await
    }
    #[tool(
        description = "Create or update your remote MCP, OpenAPI or GraphQL source. Set requiresAccount to true for services needing credentials. Link accounts in the browser; never put credentials in URLs. Local commands are operator-managed. Changing a destination removes its old linked credential.",
        annotations(destructive_hint = true)
    )]
    async fn save_source(
        &self,
        Parameters(input): Parameters<SaveSource>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.audited(
            &context,
            "save_source",
            serde_json::json!({"id":input.id,"source":"[redacted]"}),
            async {
                crate::admin::user_source(&input.source).map_err(SourceError::Input)?;
                Ok::<_, SourceError>(self.state.sources.save(input.id, input.source).await?)
            },
        )
        .await
    }
    #[tool(
        description = "Delete your source, its catalogue entries and linked credentials. Historical execution audits remain.",
        annotations(destructive_hint = true)
    )]
    async fn delete_source(
        &self,
        Parameters(input): Parameters<Id>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.audited(
            &context,
            "delete_source",
            serde_json::json!({"id":input.id}),
            self.state.sources.remove(input.id),
        )
        .await
    }
    #[tool(
        description = "Request synchronization of your source after connecting an account or changing its tools.",
        annotations(idempotent_hint = true)
    )]
    async fn sync_source(
        &self,
        Parameters(input): Parameters<Id>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.audited(
            &context,
            "sync_source",
            serde_json::json!({"id":input.id}),
            self.state.sources.resync(input.id),
        )
        .await
    }
    #[tool(
        description = "List your linked account metadata and expiry. No bearer tokens, passwords or OAuth credentials are exposed.",
        annotations(read_only_hint = true)
    )]
    async fn list_accounts(&self, context: RequestContext<RoleServer>) -> CallToolResult {
        self.audited(
            &context,
            "list_accounts",
            serde_json::json!({}),
            self.state.accounts.list(),
        )
        .await
    }
    #[tool(
        description = "Get the browser page for linking or authorizing an account for your source. The user signs in and approves the provider there; this tool never supplies a browser session or grants provider consent.",
        annotations(read_only_hint = true)
    )]
    async fn source_connection_url(
        &self,
        Parameters(input): Parameters<Id>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.audited(&context,"source_connection_url",serde_json::json!({"id":input.id}),async {
            self.state.sources.get(input.id).await?.ok_or(codemode_core::components::sources::Error::NotFound)?;
            Ok::<_,codemode_core::components::sources::Error>(serde_json::json!({"url":format!("https://{}/sources/{}/accounts",self.state.hostname,input.id)}))
        }).await
    }
}
#[tool_handler]
impl ServerHandler for Administration {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions("Manage only the signed-in user's workspaces and connected sources. Source IDs come from list_sources; workspace IDs come from list_workspaces. Credentials stay in browser account linking. Writes can remove connections and revoke access; inspect state before repeating a failed operation.")
    }
}
