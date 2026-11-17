#[path = "server/tasks.rs"]
mod tasks;
use axum::{
    Json, Router,
    extract::{Request, State as AxumState},
    http::{HeaderMap, StatusCode, header, request::Parts},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use codemode_core::{
    components::{
        auth::{Auth, Principal},
        catalogue::Catalogue,
        execution::Execution,
    },
    infrastructure::{postgres::Database, sandbox::MontySandbox},
};
use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars,
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use std::sync::Arc;
pub use tasks::Tasks;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct State {
    pub access: codemode_core::components::access::Access,
    pub mcp_tasks: Tasks,
    pub db: Database,
    pub rate_limit: Arc<codemode_core::components::auth::rate_limit::RateLimiter>,
    pub auth: Auth,
    pub catalogue: Catalogue,
    pub execution: Arc<Execution<MontySandbox>>,
    pub hostname: String,
    pub sessions: Arc<codemode_core::components::sessions::Sessions>,
    pub identity_enabled: bool,
    pub identity_routes: Option<Router>,
    pub accounts: Arc<codemode_core::components::accounts::Accounts>,
    pub sources: Arc<codemode_core::components::sources::Sources>,
    pub workspaces: Arc<codemode_core::components::workspaces::Workspaces>,
    pub stop: CancellationToken,
    pub oauth: Arc<codemode_core::components::auth::oauth::OAuth>,
}

#[derive(Clone)]
struct Gateway {
    state: State,
    manifest: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    #[serde(default)]
    query: String,
    #[serde(default)]
    detail: Detail,
    cursor: Option<String>,
}
#[derive(Default, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Detail {
    #[default]
    Brief,
    Full,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExecuteArgs {
    /// Python source, or provide scriptId instead.
    code: Option<String>,
    /// A saved script from the selected workspace. Mutually exclusive with code.
    script_id: Option<String>,
    /// JSON arguments bound as the Python `args` dictionary for a saved script.
    args: Option<serde_json::Map<String, serde_json::Value>>,
}

fn failure(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}
fn success(value: &impl serde::Serialize) -> CallToolResult {
    match serde_json::to_string(value) {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(_) => failure("could not encode tool result"),
    }
}

#[tool_router]
impl Gateway {
    #[tool(
        description = "Prefer execute for tool lookup and work in one step, using await tools.search('tool_name') inside Python. This search browses tools and saved scripts in the selected workspace. Query one tool name, or join alternatives with OR; other words must all match. Run a saved script with execute(scriptId=its id, args={...}). Each page has ten tool hits; brief gives compact input signatures, full adds complete stubs for the top three. Use the cursor for more tools or refine the query for more than 20 script matches. Blocked tools are marked and cannot be dispatched.",
        annotations(read_only_hint = true)
    )]
    async fn search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let principal = ctx
            .extensions
            .get::<Parts>()
            .and_then(|parts| parts.extensions.get::<Principal>());
        let Some(principal) = principal else {
            return failure("authentication required");
        };
        let allowed = match self
            .state
            .sources
            .allowed_sources(principal.workspace_id)
            .await
        {
            Ok(allowed) => allowed,
            Err(_) => return failure("endpoint scope unavailable"),
        };
        match self
            .state
            .catalogue
            .search_scoped(
                &args.query,
                matches!(args.detail, Detail::Full),
                args.cursor.as_deref(),
                allowed.as_deref(),
                Some(principal.workspace_id),
            )
            .await
        {
            Ok(page) => {
                match codemode_core::components::scripts::Scripts::new(self.state.db.clone())
                    .search(principal.workspace_id, &args.query)
                    .await
                {
                    Ok(scripts) => {
                        #[derive(serde::Serialize)]
                        struct Results {
                            #[serde(flatten)]
                            tools: codemode_core::components::catalogue::Page,
                            scripts: Vec<codemode_core::components::scripts::Match>,
                        }
                        success(&Results {
                            tools: page,
                            scripts,
                        })
                    }
                    Err(error) => failure(error.to_string()),
                }
            }
            Err(error) => failure(error.to_string()),
        }
    }

    #[tool(
        description = "Do the whole task in one execute. Call listed tools directly; invalid arguments raise ToolError with the signature. Provide Python code or a saved scriptId with args.\nDiscover with await tools.search('tool_name') only when needed; use OR between alternatives. print(help(tool)) gives full signatures.\nUse asyncio.gather(*calls) without keyword arguments for independent calls and .all(max_pages=20, _fields=[...]) for rows; compute the answer locally.\nUse memo.get(key) / memo.set(key, value), not indexing: 64 KiB per session for non-sensitive IDs/small summaries, never datasets. Return only the answer.\nUse IDs returned by list tools, never guessed titles or IDs. Avoid generators and next(list). Catch ToolError and PermissionError.\nLimits: 60 s, 64 MB, 100 calls, 64 KB source; no inheritance, generators, files, network or secrets."
    )]
    async fn execute(
        &self,
        Parameters(args): Parameters<ExecuteArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let principal = ctx
            .extensions
            .get::<Parts>()
            .and_then(|p| p.extensions.get::<Principal>())
            .cloned();
        let Some(principal) = principal else {
            return failure("authentication required");
        };
        let run =
            async {
                match (args.code, args.script_id, args.args) {
                    (Some(code), None, None) => self.state.execution.execute(principal, code).await,
                    (None, Some(id), script_args) => match uuid::Uuid::parse_str(&id) {
                        Ok(id) => {
                            self.state
                                .execution
                                .execute_script(principal, id, script_args.unwrap_or_default())
                                .await
                        }
                        Err(_) => return Err("scriptId must be a UUID".to_owned()),
                    },
                    _ => return Err(
                        "provide exactly one of code or scriptId; args is only valid with scriptId"
                            .to_owned(),
                    ),
                }
                .map_err(|error| error.to_string())
            };
        tokio::select! {
            _=ctx.ct.cancelled()=>failure("execution cancelled"),
            result=run=>match result {
                Ok(result) if result.error.is_none()=>success(&result),
                Ok(result)=> {let mut response=success(&result);response.is_error=Some(true);response},
                Err(error)=>failure(error.to_string()),
            }
        }
    }
}

#[tool_handler]
impl ServerHandler for Gateway {
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let mut tools = Self::tool_router().list_all();
        for tool in &mut tools {
            if tool.name == "execute" {
                tool.description = Some(
                    format!(
                        "{}\n{}",
                        tool.description.as_deref().unwrap_or_default(),
                        self.manifest
                    )
                    .into(),
                );
            }
        }
        let mut result = rmcp::model::ListToolsResult::with_all_items(tools);
        if context
            .protocol_version()
            .is_some_and(|v| v >= rmcp::model::ProtocolVersion::V_2026_07_28)
        {
            result.ttl_ms = Some(0);
            result.cache_scope = Some(rmcp::model::CacheScope::Private);
        }
        Ok(result)
    }
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        mut context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        use rmcp::{
            handler::server::tool::ToolCallContext,
            model::{CallToolResponse, CreateTaskResult},
            task_manager::TaskExit,
        };
        let capable = context
            .protocol_version()
            .is_some_and(|version| version >= rmcp::model::ProtocolVersion::V_2026_07_28)
            && context
                .client_capabilities()
                .is_some_and(|caps| caps.supports_tasks());
        if capable && request.name.as_ref() == "execute" {
            let principal = principal(&context)?;
            let tenant = principal.tenant_id;
            // A task outlives its originating HTTP request; its SDK cancellation still drops the execution future.
            context.ct = self.state.stop.child_token();
            let gateway = self.clone();
            let task = self.state.mcp_tasks.spawn(&principal, move |task| Box::pin(async move {
                let router = Self::tool_router();
                gateway.state.access.run(tenant, async { tokio::select! {
                    biased;
                    _ = task.cancelled() => Err(TaskExit::Cancelled),
                    result = router.call(ToolCallContext::new(&gateway, request, context)) => match result? {
                        CallToolResponse::Complete(result) => Ok(result),
                        _ => Err(rmcp::ErrorData::internal_error("unexpected nested tool response", None).into()),
                    }
                }}).await.unwrap_or_else(|error| Err(rmcp::ErrorData::internal_error(error.to_string(), None).into()))
            }))?;
            Ok(CallToolResponse::Task(CreateTaskResult::new(task)))
        } else {
            let tenant = principal(&context)?.tenant_id;
            self.state
                .access
                .run(
                    tenant,
                    Self::tool_router().call(ToolCallContext::new(self, request, context)),
                )
                .await
                .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))?
        }
    }
    async fn get_task(
        &self,
        request: rmcp::model::GetTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetTaskResult, rmcp::ErrorData> {
        Ok(rmcp::model::GetTaskResult::new(
            self.state
                .mcp_tasks
                .get(&principal(&context)?, &request.task_id)?,
        ))
    }
    async fn update_task(
        &self,
        request: rmcp::model::UpdateTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), rmcp::ErrorData> {
        self.state.mcp_tasks.update(
            &principal(&context)?,
            &request.task_id,
            request.input_responses,
        )
    }
    async fn cancel_task(
        &self,
        request: rmcp::model::CancelTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), rmcp::ErrorData> {
        self.state
            .mcp_tasks
            .cancel(&principal(&context)?, &request.task_id)
    }
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_tasks().build())
            .with_server_info(Implementation::new("codemode", option_env!("CODEMODE_BUILD_ID").unwrap_or(env!("CARGO_PKG_VERSION"))))
            .with_instructions("This /mcp resource is bound to your selected workspace. See the execute description for tool signatures and usage. When Codemode covers the same apps as ChatGPT connectors, use features.apps=false or disconnect duplicate connectors so the client uses the intended gateway.")
    }
}

fn principal(context: &RequestContext<RoleServer>) -> Result<Principal, rmcp::ErrorData> {
    context
        .extensions
        .get::<Parts>()
        .and_then(|parts| parts.extensions.get::<Principal>())
        .cloned()
        .ok_or_else(|| rmcp::ErrorData::invalid_request("authentication required", None))
}

async fn mcp_request(AxumState(state): AxumState<State>, request: Request) -> Response {
    let Some(principal) = request.extensions().get::<Principal>() else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let manifest = match state.catalogue.manifest(principal.workspace_id).await {
        Ok(manifest) => manifest,
        Err(error) => {
            tracing::error!(?error, "workspace manifest unavailable");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "workspace catalogue unavailable",
            )
                .into_response();
        }
    };
    let config = transport(&state);
    let factory = state.clone();
    let service = StreamableHttpService::new(
        move || {
            Ok(Gateway {
                state: factory.clone(),
                manifest: manifest.clone(),
            })
        },
        Arc::new(LocalSessionManager::default()),
        config,
    );
    service.handle(request).await.into_response()
}

pub(crate) fn transport(state: &State) -> StreamableHttpServerConfig {
    StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_cancellation_token(state.stop.child_token())
        .with_allowed_hosts([state.hostname.clone()])
        .with_allowed_origins([format!("https://{}:443", state.hostname)])
}

#[path = "server/tenancy.rs"]
mod tenancy;

pub fn router(state: State) -> Router {
    tenancy::router(state)
}
fn routes(state: State) -> Router {
    let mcp = Router::new()
        .route("/mcp", any(mcp_request))
        .route("/mcp/", any(mcp_request))
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));
    Router::new()
        .route("/healthz", get(health))
        .route(
            "/.well-known/oauth-protected-resource",
            get(resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp-admin",
            get(resource_metadata),
        )
        .merge(mcp)
        .merge(crate::mcp_admin::routes(state.clone()))
        .with_state(state.clone())
        .merge(state.identity_routes.clone().unwrap_or_default())
        .merge(crate::oauth::router(crate::oauth::State {
            oauth: state.oauth.clone(),
            hostname: state.hostname.clone(),
        }))
        .merge(crate::admin::router(crate::admin::State {
            scripts: codemode_core::components::scripts::Scripts::new(state.db.clone()),
            log: codemode_core::components::execution::log::Log::new(state.db.clone()),
            agent_sessions: codemode_core::components::agent_sessions::AgentSessions::new(
                state.db.clone(),
            ),
            auth: state.auth.clone(),
            sessions: state.sessions.clone(),
            identity_enabled: state.identity_enabled,
            access: state.access.clone(),
            accounts: state.accounts.clone(),
            sources: state.sources.clone(),
            hostname: state.hostname.clone(),
            workspaces: state.workspaces.clone(),
            oauth: state.oauth.clone(),
        }))
        .fallback(crate::webui::handler(state.hostname.clone()))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(128 * 1024))
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(65),
        ))
}

async fn resource_metadata(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .filter(|_| headers.get_all(header::HOST).iter().count() == 1)
        .and_then(codemode_core::components::workspaces::authority);
    if host.as_deref() != Some(&state.hostname) {
        return (StatusCode::NOT_FOUND, "unknown resource").into_response();
    }
    let admin = request.uri().path().ends_with("/mcp-admin");
    let resource = if admin { "/mcp-admin" } else { "/mcp" };
    Json(serde_json::json!({"resource":format!("https://{}{resource}",state.hostname),"authorization_servers":[format!("https://{}",state.hostname)],"bearer_methods_supported":["header"],"scopes_supported":[if admin {"admin"}else{"mcp"}],"resource_name":if admin {"Codemode Administration"}else{"Codemode Workspace"}})).into_response()
}

async fn health(AxumState(state): AxumState<State>) -> Response {
    if state.db.healthy().await {
        Json(serde_json::json!({
            "status":"ok",
            "build":option_env!("CODEMODE_BUILD_ID").unwrap_or("development"),
            "schemaVersion":state.db.schema_version(),
            "identityAccessEnforced":state.access.enforced(),
        }))
        .into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response()
    }
}

pub(crate) async fn authenticate(
    AxumState(state): AxumState<State>,
    mut request: Request,
    next: Next,
) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .filter(|_| request.headers().get_all(header::HOST).iter().count() == 1)
        .unwrap_or_default();
    let Some(host) = codemode_core::components::workspaces::authority(host) else {
        return (StatusCode::NOT_FOUND, "unknown endpoint").into_response();
    };
    if host != state.hostname {
        return (StatusCode::NOT_FOUND, "unknown resource").into_response();
    }
    let resource = request.uri().path().trim_end_matches('/').to_owned();
    let Ok(canonical_host) = axum::http::HeaderValue::from_str(&host) else {
        return (StatusCode::BAD_REQUEST, "invalid hostname").into_response();
    };
    request.headers_mut().insert(header::HOST, canonical_host);
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .filter(|_| {
            request
                .headers()
                .get_all(header::AUTHORIZATION)
                .iter()
                .count()
                == 1
        })
        .and_then(|s| s.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim_start_matches(' '));
    if let Some(token) = token {
        match state.auth.authenticate(token, &host).await {
            Ok(Some(principal)) if principal.resource == resource => {
                match state.workspaces.get(principal.workspace_id).await {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        return (StatusCode::UNAUTHORIZED, "workspace is no longer available")
                            .into_response();
                    }
                    Err(error) => {
                        tracing::error!(error=?error,"workspace lookup failed");
                        return (StatusCode::SERVICE_UNAVAILABLE, "workspace unavailable")
                            .into_response();
                    }
                }
                let routed_call = request
                    .headers()
                    .get("Mcp-Method")
                    .is_some_and(|v| v == "tools/call");
                let (parts, body) = request.into_parts();
                let bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        return (StatusCode::PAYLOAD_TOO_LARGE, "MCP request exceeds 1 MiB")
                            .into_response();
                    }
                };
                let is_call = routed_call
                    || serde_json::from_slice::<serde_json::Value>(&bytes).is_ok_and(|message| {
                        message.get("method").and_then(serde_json::Value::as_str)
                            == Some("tools/call")
                    });
                request = Request::from_parts(parts, axum::body::Body::from(bytes));
                if is_call {
                    let admission = if resource == "/mcp" {
                        let session =
                            match codemode_core::components::agent_sessions::AgentSessions::new(
                                state.db.clone(),
                            )
                            .touch(&principal)
                            .await
                            {
                                Ok(session) => session,
                                Err(error) => {
                                    tracing::error!(error=%error, "agent session admission failed");
                                    return (
                                        StatusCode::SERVICE_UNAVAILABLE,
                                        "agent session unavailable",
                                    )
                                        .into_response();
                                }
                            };
                        state.rate_limit.check_session(
                            session,
                            principal.token_id,
                            principal.workspace_id,
                        )
                    } else {
                        state
                            .rate_limit
                            .check(principal.token_id, principal.workspace_id)
                    };
                    if let Err(seconds) = admission {
                        let mut response=(StatusCode::TOO_MANY_REQUESTS,Json(serde_json::json!({"error":"MCP tool-call rate limit exceeded; retry after the indicated delay"}))).into_response();
                        if let Ok(value) = axum::http::HeaderValue::from_str(&seconds.to_string()) {
                            response.headers_mut().insert(header::RETRY_AFTER, value);
                        }
                        return response;
                    }
                }
                request.extensions_mut().insert(principal);
                return next.run(request).await;
            }
            Err(error) => {
                if let codemode_core::components::auth::Error::Access(error) = error {
                    return crate::access::response(error);
                }
                tracing::error!(error=%error,"token authentication failed");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "authentication unavailable",
                )
                    .into_response();
            }
            _ => {}
        }
    }
    let mut response = (StatusCode::UNAUTHORIZED, "valid bearer token required").into_response();
    if let Ok(challenge) = axum::http::HeaderValue::from_str(&format!(
        "Bearer resource_metadata=\"https://{host}/.well-known/oauth-protected-resource{resource}\", scope=\"{}\"",
        if resource == "/mcp-admin" {
            "admin"
        } else {
            "mcp"
        }
    )) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, challenge);
    }
    response
}
