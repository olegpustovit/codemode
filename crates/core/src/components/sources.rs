mod store;
pub use super::broker::Tool;
use super::{
    accounts::Accounts,
    broker::{Broker, Credentials, Target},
    catalogue::{Catalogue, Definition, stubs},
    workspaces::Workspaces,
};
use crate::infrastructure::{
    postgres::{Database, Error as DbError},
    upstream::{Replies, Stdio},
};
use futures_util::{StreamExt, stream};
use rmcp::{
    ClientHandler, ServiceExt,
    model::{ClientConfig, PaginatedRequestParams, ProtocolVersion},
    schemars,
    service::{NotificationContext, RoleClient, RunningService},
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};
pub use store::{Display, Record, SourceInput};
use tokio::sync::{Notify, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct Source {
    pub slug: String,
    #[serde(default, rename = "requiresAccount", alias = "requires_account")]
    pub requires_account: bool,
    #[serde(flatten)]
    pub transport: Transport,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transport {
    McpHttp {
        url: String,
        #[serde(default)]
        bearer_env: Option<String>,
    },
    #[serde(rename = "openapi")]
    OpenApi {
        url: String,
        #[serde(default, rename = "serverUrl", alias = "server_url")]
        server_url: Option<String>,
    },
    Graphql {
        url: String,
    },
    McpStdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub sources: Vec<Source>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid sources configuration: {0}")]
    Config(String),
    #[error("{0}")]
    Display(&'static str),
    #[error("source connection or synchronization failed")]
    Upstream,
    #[error("OpenAPI document loading is busy; retry shortly")]
    Busy,
    #[error(transparent)]
    Policy(#[from] super::policies::Error),
    #[error(transparent)]
    OpenApi(#[from] super::openapi::Error),
    #[error("source changed during synchronization")]
    Changed,
    #[error("source does not exist")]
    NotFound,
    #[error("source slug is already in use")]
    Conflict,
    #[error(transparent)]
    Account(#[from] super::accounts::Error),
    #[error(transparent)]
    Endpoint(#[from] super::workspaces::Error),
    #[error(transparent)]
    Database(#[from] DbError),
}

impl Config {
    pub async fn read(path: &Path) -> Result<Self, Error> {
        let text = tokio::fs::read_to_string(path)
            .await
            .map_err(|_| Error::Config("cannot read file".into()))?;
        let config: Self = toml::from_str(&text).map_err(|_| {
            Error::Config("invalid TOML; check the source field names and types".into())
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.sources.len() > 64 {
            return Err(Error::Config("maximum 64 sources".into()));
        }
        let mut slugs = std::collections::BTreeSet::new();
        for source in &self.sources {
            if source.slug.len() > 64
                || source.slug.is_empty()
                || !source
                    .slug
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                || !slugs.insert(source.slug.clone())
            {
                return Err(Error::Config("source slugs must be distinct and contain 1..64 letters, digits, underscores or dashes".into()));
            }
            match &source.transport {
                Transport::McpHttp { url, bearer_env } => {
                    let parsed = url::Url::parse(url)
                        .map_err(|_| Error::Config("invalid HTTP source URL".into()))?;
                    if !["http", "https"].contains(&parsed.scheme())
                        || parsed.host_str().is_none()
                        || !parsed.username().is_empty()
                        || parsed.password().is_some()
                        || parsed.fragment().is_some()
                        || parsed.query().is_some()
                    {
                        return Err(Error::Config("HTTP source URL needs an HTTP(S) host and no credentials, query or fragment; use bearer_env".into()));
                    }
                    if parsed.path().ends_with("/sse") {
                        return Err(Error::Config("legacy SSE sources are unsupported; use the Streamable HTTP /mcp endpoint".into()));
                    }
                    if bearer_env.as_ref().is_some_and(|v| !env_name(v)) {
                        return Err(Error::Config("invalid bearer environment reference".into()));
                    }
                }
                Transport::Graphql { url } => {
                    super::openapi::server_url(url)?;
                }
                Transport::OpenApi { url, server_url } => {
                    super::openapi::server_url(url)?;
                    if let Some(server) = server_url {
                        super::openapi::server_url(server)?;
                    }
                }
                Transport::McpStdio { command, args, env } => {
                    if !Path::new(command).is_absolute()
                        || args.len() > 128
                        || env.len() > 64
                        || env.iter().any(|(k, v)| !env_name(k) || !env_name(v))
                    {
                        return Err(Error::Config("stdio needs an absolute executable, at most 128 arguments, and an explicit environment-reference allowlist".into()));
                    }
                }
            }
        }
        Ok(())
    }
}

fn env_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .enumerate()
            .all(|(i, c)| c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

struct Connected {
    client: RunningService<RoleClient, Handler>,
    replies: Replies,
    credentials: Credentials,
    revision: i64,
}

#[derive(Clone)]
pub struct Sources {
    access: super::access::Access,
    tenant: Uuid,
    db: Database,
    catalogue: Catalogue,
    accounts: Accounts,
    broker: Broker,
    policies: super::policies::Policies,
    workspaces: Workspaces,
    tools: Arc<RwLock<BTreeMap<String, Tool>>>,
    changed: Arc<Notify>,
    ready: tokio::sync::watch::Sender<bool>,
    documents: Arc<tokio::sync::Semaphore>,
}

#[derive(Clone)]
pub struct Handler {
    changed: Arc<Notify>,
}
impl ClientHandler for Handler {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default().with_protocol_version(ProtocolVersion::V_2025_11_25)
    }
    async fn on_tool_list_changed(&self, _: NotificationContext<RoleClient>) {
        self.changed.notify_one();
    }
}

impl Sources {
    pub fn new(
        db: Database,
        catalogue: Catalogue,
        accounts: Accounts,
        workspaces: Workspaces,
    ) -> Self {
        let policies = super::policies::Policies::new(db.clone());
        Self {
            access: Default::default(),
            tenant: Uuid::nil(),
            broker: Broker::new(
                db.clone(),
                accounts.clone(),
                workspaces.clone(),
                policies.clone(),
            ),
            policies,
            workspaces,
            accounts,
            db,
            catalogue,
            tools: Arc::new(RwLock::new(BTreeMap::new())),
            changed: Arc::new(Notify::new()),
            ready: tokio::sync::watch::channel(false).0,
            documents: Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }

    async fn load_openapi(
        &self,
        url: String,
        server: Option<String>,
        namespace: String,
    ) -> Result<super::openapi::Loaded, Error> {
        let permit = self
            .documents
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        Ok(super::openapi::load(url, server, namespace, permit).await?)
    }

    pub async fn ready(&self) {
        let mut ready = self.ready.subscribe();
        let _ = ready.wait_for(|ready| *ready).await;
    }

    pub fn wake(&self) {
        self.changed.notify_one();
    }

    pub async fn dispatch(
        &self,
        execution: Uuid,
        seq: usize,
        call: crate::infrastructure::sandbox::PendingCall,
        timeout: Duration,
        workspace: Uuid,
    ) -> (u32, crate::infrastructure::sandbox::CallResult) {
        let tool = if call.path == super::broker::UNKNOWN_PATH {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Lookup {
                path: String,
            }
            match serde_json::from_str::<Lookup>(call.args.get()) {
                Ok(input) if !input.path.is_empty() && input.path.len() <= 1024 => {
                    Err(self.unknown(&input.path, workspace).await)
                }
                _ => Err("unknown-tool lookup requires a path of 1–1024 bytes".into()),
            }
        } else {
            match self.lookup(&call.path).await {
                Some(tool) => Ok(tool),
                None if matches!(
                    call.path.as_str(),
                    super::broker::SEARCH_PATH | super::broker::MEMO_PATH
                ) =>
                {
                    Err(String::new())
                }
                None => Err(self.unknown(&call.path, workspace).await),
            }
        };
        self.broker
            .dispatch(execution, seq, call, tool, timeout, workspace)
            .await
    }

    pub fn policies(&self) -> &super::policies::Policies {
        &self.policies
    }

    pub async fn tool_names(&self, source: Uuid) -> Vec<String> {
        self.tools
            .read()
            .await
            .values()
            .filter(|tool| tool.source_id == source)
            .map(|tool| tool.definition.path.clone())
            .collect()
    }

    pub async fn lookup(&self, path: &str) -> Option<Tool> {
        self.tools.read().await.get(path).cloned()
    }

    pub async fn allowed_sources(&self, workspace: Uuid) -> Result<Option<Vec<Uuid>>, Error> {
        Ok(self.workspaces.sources(workspace).await?)
    }

    pub async fn prelude(
        &self,
        allowed: Option<&[Uuid]>,
        workspace: Uuid,
    ) -> Result<String, Error> {
        let scope = self.workspaces.tool_scope(workspace).await?;
        let registry = self.tools.read().await;
        let mut groups: BTreeMap<&str, Vec<&Tool>> = BTreeMap::new();
        for (path, tool) in registry.iter() {
            if allowed.is_some_and(|ids| !ids.contains(&tool.source_id)) {
                continue;
            }
            if let Some((source, _)) = path.split_once('.') {
                groups.entry(source).or_default().push(tool);
            }
        }
        let mut code = String::from(include_str!("sources/prelude.py"));
        code.push_str("\n_sources = {}\n");
        for (source, tools) in &groups {
            code.push_str(&format!(
                "_sources[{}] = _source_proxy({})\n",
                stubs::quote(source),
                stubs::quote(source)
            ));
            for tool in tools {
                let name = tool
                    .definition
                    .path
                    .split_once('.')
                    .map(|(_, n)| n)
                    .unwrap_or_default();
                let mut aliases = BTreeMap::new();
                if let Some(props) = tool
                    .definition
                    .input_schema
                    .get("properties")
                    .and_then(Value::as_object)
                {
                    for (original, alias) in stubs::names(props.keys().map(String::as_str)) {
                        aliases.insert(alias, original);
                    }
                }
                let aliases = serde_json::to_string(&aliases).unwrap_or_else(|_| "{}".into());
                let in_scope = scope
                    .as_ref()
                    .is_none_or(|paths| paths.contains(&tool.definition.path));
                let full = if in_scope {
                    stubs::generate(
                        &tool.definition.path,
                        &tool.definition.description,
                        &tool.definition.input_schema,
                        tool.definition.output_schema.as_ref(),
                    )
                    .full
                } else {
                    "tool is outside workspace tool scope".into()
                };
                let pagination = stubs::pagination(
                    &tool.definition.input_schema,
                    tool.definition.output_schema.as_ref(),
                )
                .map(|paths| {
                    format!(
                        "json.loads({})",
                        stubs::quote(&serde_json::to_string(&paths).unwrap_or_default())
                    )
                })
                .unwrap_or_else(|| "None".into());
                code.push_str(&format!(
                    "_sources[{}].{name} = _Tool({}, {aliases}, {}, {pagination})\n",
                    stubs::quote(source),
                    stubs::quote(&tool.definition.path),
                    stubs::quote(&full)
                ));
            }
        }
        code.push_str("tools = _source_proxy('')\ntools.search = _search\n");
        for source in groups.keys() {
            code.push_str(&format!(
                "tools.{source} = _sources[{}]\n",
                stubs::quote(source)
            ));
        }
        Ok(code)
    }

    pub async fn unknown(&self, path: &str, workspace: Uuid) -> String {
        let allowed = match self.allowed_sources(workspace).await {
            Ok(allowed) => allowed,
            Err(_) => return "endpoint scope unavailable".into(),
        };
        let blocked = match self.policies.blocked(workspace).await {
            Ok(paths) => paths.into_iter().collect::<std::collections::BTreeSet<_>>(),
            Err(_) => return "endpoint policy unavailable".into(),
        };
        let paths: Vec<_> = self
            .tools
            .read()
            .await
            .iter()
            .filter(|(path, tool)| {
                !blocked.contains(*path)
                    && allowed
                        .as_ref()
                        .is_none_or(|ids| ids.contains(&tool.source_id))
            })
            .map(|(path, _)| path.clone())
            .collect();
        let source = path.split('.').next().unwrap_or_default();
        let mut candidates = paths
            .iter()
            .filter(|p| p.split_once('.').is_some_and(|(name, _)| name == source))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            candidates = paths.iter().collect();
        }
        candidates
            .sort_by_cached_key(|candidate| (name_distance(path, candidate), (*candidate).clone()));
        let matches = candidates.into_iter().take(3).cloned().collect::<Vec<_>>();
        if matches.is_empty() {
            if let Ok(Some(id)) =
                sqlx::query_scalar!("SELECT id FROM sources WHERE python_name=$1 AND ($2::uuid[] IS NULL OR id=ANY($2))", source,allowed.as_deref())
                    .fetch_optional(&self.db.0)
                    .await
            {
                return format!(
                    "tool {path} is unavailable; link the account or retry synchronization at {}",
                    self.broker.account_link(id)
                );
            }
        }
        if matches.is_empty() {
            return super::broker::unavailable(path, workspace);
        }
        format!("unknown tool {path}; did you mean: {}", matches.join(", "))
    }

    pub fn with_access(mut self, access: super::access::Access, tenant: Uuid) -> Self {
        self.access = access;
        self.tenant = tenant;
        self
    }

    pub async fn run(&self, stop: CancellationToken) {
        if !self.access.enforced() {
            self.run_epoch(stop).await;
            return;
        }
        loop {
            if stop.is_cancelled() {
                return;
            }
            if let Ok(mut lease) = self.access.watch(self.tenant).await {
                let epoch = stop.child_token();
                let future = self.run_epoch(epoch.clone());
                tokio::pin!(future);
                tokio::select! { biased;
                    _=stop.cancelled()=>{},
                    _=lease.stopped()=>{},
                    _=&mut future=>{ lease.close().await; return; },
                }
                epoch.cancel();
                // Join source cleanup: close persistent HTTP/stdio transports,
                // including clients still being synchronized, before retrying.
                future.await;
                lease.close().await;
            }
            tokio::select! { biased;
                _=stop.cancelled()=>return,
                _=tokio::time::sleep(Duration::from_secs(5))=>{},
            }
        }
    }

    async fn run_epoch(&self, stop: CancellationToken) {
        self.ready.send_replace(false);
        let mut connections: BTreeMap<Uuid, Connected> = BTreeMap::new();
        let mut timer = tokio::time::interval(Duration::from_secs(60));
        // Coalesce the initial timer tick with source changes queued before startup.
        timer.tick().await;
        self.changed.notify_one();
        'running: loop {
            tokio::select! {biased; _=stop.cancelled()=>break, _=timer.tick()=>{}, _=self.changed.notified()=>{}}
            let records = match self.list().await {
                Ok(records) => records,
                Err(error) => {
                    tracing::error!(error=?error,"load managed sources failed");
                    continue;
                }
            };
            let revisions: BTreeMap<_, _> = records
                .iter()
                .map(|record| (record.id, record.revision))
                .collect();
            self.tools
                .write()
                .await
                .retain(|_, tool| revisions.get(&tool.source_id) == Some(&tool.revision));
            let jobs: Vec<_> = records
                .into_iter()
                .map(|record| {
                    let existing = connections.remove(&record.id);
                    let alias = record.python_name.clone();
                    (record, existing, alias)
                })
                .collect();
            stream::iter(std::mem::take(&mut connections).into_values())
                .for_each_concurrent(16, Self::close)
                .await;
            let mut pending = stream::iter(jobs)
                .map(|(record, existing, alias)| self.sync_one(record, existing, alias, &stop))
                .buffer_unordered(4);
            loop {
                tokio::select! {
                    biased;
                    _=stop.cancelled()=>{
                        while let Some((_, connection)) = pending.next().await {
                            if let Some(connection) = connection { Self::close(connection).await; }
                        }
                        break 'running;
                    },
                    next=pending.next()=>match next {
                        Some((id,Some(connection)))=>{connections.insert(id,connection);},
                        Some((_,None))=>{},
                        None=>break,
                    }
                }
            }
            self.ready.send_replace(true);
        }
        self.tools.write().await.clear();
        self.ready.send_replace(false);
        stream::iter(connections.into_values())
            .for_each_concurrent(16, Self::close)
            .await;
    }

    async fn close(mut connection: Connected) {
        let _ = connection
            .client
            .close_with_timeout(Duration::from_secs(1))
            .await;
    }

    async fn sync_one(
        &self,
        record: Record,
        mut connection: Option<Connected>,
        alias: String,
        stop: &CancellationToken,
    ) -> (Uuid, Option<Connected>) {
        if record.status == "needs_link" {
            if let Some(old) = connection.take() {
                Self::close(old).await;
            }
            return (record.id, None);
        }
        if connection
            .as_ref()
            .is_some_and(|c| c.revision != record.revision)
        {
            if let Some(old) = connection.take() {
                Self::close(old).await;
            }
        }
        let result = tokio::select! {
            biased;
            _=stop.cancelled()=>None,
            result=tokio::time::timeout(Duration::from_secs(20),async {
                if let Transport::OpenApi {url,server_url}=&record.source.transport {
                    let server=server_url.as_ref().ok_or(Error::Config("OpenAPI server is not pinned".into()))?;
                    let loaded=self.load_openapi(url.clone(),Some(server.clone()),alias.clone()).await?;
                    let compiled=loaded.operations.into_iter().map(|operation| {Ok(Tool {
                        source_id:record.id,revision:record.revision,definition:operation.definition.clone(),validator:operation.validator.clone(),
                        omit_null:Arc::new(super::broker::optional_nonnullable(&operation.definition.input_schema).map_err(|_|Error::Upstream)?),
                        target:Target::OpenApi {operation:Arc::new(operation),client:loaded.client.clone(),server:loaded.server.clone(),required:record.source.requires_account},
                    })}).collect::<Result<Vec<_>,Error>>()?;
                    return self.publish(&record,compiled).await;
                }
                if let Transport::Graphql {url}=&record.source.transport {
                    let client=super::graphql::client().map_err(|_|Error::Upstream)?;
                    let compiled=[super::graphql::Kind::Query,super::graphql::Kind::Mutation].into_iter().map(|kind| {
                        let definition=kind.definition(&alias);
                        let omit_null=Arc::new(super::broker::optional_nonnullable(&definition.input_schema).map_err(|_|Error::Upstream)?);
                        let validator=jsonschema::validator_for(&definition.input_schema).map_err(|_|Error::Config("invalid GraphQL input schema".into()))?;
                        Ok(Tool {source_id:record.id,revision:record.revision,definition,validator:Arc::new(validator),omit_null,target:Target::Graphql {kind,client:client.clone(),url:url.clone(),required:record.source.requires_account}})
                    }).collect::<Result<Vec<_>,Error>>()?;
                    return self.publish(&record,compiled).await;
                }
                if connection.is_none() {connection=Some(self.connect(&record).await?);}
                self.synchronize(&record,&alias,connection.as_ref().ok_or(Error::Upstream)?).await
            })=>Some(result.unwrap_or(Err(Error::Upstream))),
        };
        match result {
            Some(Ok(())) => return (record.id, connection),
            Some(Err(Error::Changed)) => self.wake(),
            Some(Err(error)) => {
                let message = error.to_string();
                if let Err(error) = sqlx::query!(
                    "UPDATE sources SET status='error',last_error=$3 WHERE id=$1 AND revision=$2",
                    record.id,
                    record.revision,
                    message
                )
                .execute(&self.db.0)
                .await
                {
                    tracing::error!(error=%error,"source status update failed");
                }
                tracing::warn!(source=%record.source.slug,error=?error,"source synchronization failed; retaining last good catalogue");
            }
            None => {}
        }
        if let Some(connection) = connection {
            Self::close(connection).await;
        }
        (record.id, None)
    }

    async fn connect(&self, record: &Record) -> Result<Connected, Error> {
        let replies = Replies::default();
        let handler = Handler {
            changed: self.changed.clone(),
        };
        let (client, credentials) = match &record.source.transport {
            Transport::McpHttp { url, .. } => {
                let http = self
                    .broker
                    .http(record.id, record.source.requires_account, replies.clone())
                    .map_err(|_| Error::Upstream)?;
                let client = handler
                    .serve(StreamableHttpClientTransport::with_client(
                        http,
                        StreamableHttpClientTransportConfig::with_uri(url.clone()),
                    ))
                    .await
                    .map_err(|_| Error::Upstream)?;
                (client, Credentials::Http)
            }
            Transport::OpenApi { .. } | Transport::Graphql { .. } => {
                return Err(Error::Config(
                    "HTTP API sources do not use MCP connections".into(),
                ));
            }
            Transport::McpStdio { command, args, .. } => {
                let environment = self.broker.environment(record.id).await?;
                if record.source.requires_account && environment.is_empty() {
                    return Err(Error::Upstream);
                }
                let mut command = tokio::process::Command::new(command);
                command.args(args).env_clear().kill_on_drop(true);
                for (name, value) in &environment {
                    command.env(name, value.expose_secret());
                }
                let secrets = Arc::new(environment.into_values().collect());
                let client = handler
                    .serve(Stdio::new(command, replies.clone()).map_err(|_| Error::Upstream)?)
                    .await
                    .map_err(|_| Error::Upstream)?;
                (client, Credentials::Stdio(secrets))
            }
        };
        Ok(Connected {
            client,
            replies,
            credentials,
            revision: record.revision,
        })
    }

    async fn synchronize(
        &self,
        record: &Record,
        alias: &str,
        client: &Connected,
    ) -> Result<(), Error> {
        let mut tools = Vec::new();
        let mut cursor = None;
        for page in 0..100 {
            let result = client
                .client
                .list_tools(Some({
                    let mut params = PaginatedRequestParams::default();
                    params.cursor = cursor;
                    params
                }))
                .await
                .map_err(|_| Error::Upstream)?;
            tools.extend(result.tools);
            if tools.len() > 10_000 {
                return Err(Error::Upstream);
            }
            cursor = result.next_cursor;
            if cursor.is_none() {
                break;
            }
            if page == 99 {
                return Err(Error::Upstream);
            }
        }
        let aliases = stubs::names(tools.iter().map(|t| t.name.as_ref()));
        let mut compiled = Vec::new();
        for tool in tools {
            let input_schema = Value::Object((*tool.input_schema).clone());
            let validator = jsonschema::options()
                .offline()
                .build(&input_schema)
                .map_err(|_| Error::Upstream)?;
            let hints = tool.annotations.as_ref();
            let definition = Definition {
                path: format!("{alias}.{}", aliases[tool.name.as_ref()]),
                upstream_name: tool.name.to_string(),
                description: tool.description.unwrap_or_default().to_string(),
                input_schema,
                output_schema: tool.output_schema.map(|s| Value::Object((*s).clone())),
                read_only: hints.and_then(|h| h.read_only_hint).unwrap_or(false),
                destructive: hints.and_then(|h| h.destructive_hint).unwrap_or(false),
            };
            let omit_null = Arc::new(
                super::broker::optional_nonnullable(&definition.input_schema)
                    .map_err(|_| Error::Upstream)?,
            );
            compiled.push(Tool {
                source_id: record.id,
                revision: record.revision,
                definition,
                validator: Arc::new(validator),
                omit_null,
                target: Target::Mcp {
                    peer: client.client.peer().clone(),
                    replies: client.replies.clone(),
                    credentials: client.credentials.clone(),
                },
            });
        }
        self.publish(record, compiled).await
    }
    async fn publish(&self, record: &Record, compiled: Vec<Tool>) -> Result<(), Error> {
        let definitions: Vec<_> = compiled.iter().map(|t| t.definition.clone()).collect();
        if !self
            .catalogue
            .replace(record.id, &definitions, record.revision)
            .await?
        {
            return Err(Error::Changed);
        }
        let mut registry = self.tools.write().await;
        registry.retain(|_, tool| tool.source_id != record.id);
        for tool in compiled {
            registry.insert(tool.definition.path.clone(), tool);
        }
        Ok(())
    }
}

fn name_distance(left: &str, right: &str) -> usize {
    // Tool paths are ASCII identifiers; bound work even for an untrusted callback path.
    let left = &left.as_bytes()[..left.len().min(256)];
    let right = &right.as_bytes()[..right.len().min(256)];
    let mut row: Vec<_> = (0..=right.len()).collect();
    for (i, a) in left.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, b) in right.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(a != b));
            diagonal = above;
        }
    }
    row[right.len()]
}

#[cfg(test)]
mod tests {
    #[test]
    fn closest_names_handle_insertions_deletions_and_substitutions() {
        assert_eq!(
            super::name_distance("nudge.list_issue", "nudge.list_issues"),
            1
        );
        assert_eq!(super::name_distance("abc", "axc"), 1);
        assert_eq!(super::name_distance("", "abc"), 3);
        assert_eq!(super::name_distance("same", "same"), 0);
        assert!(super::name_distance("nudge.list_issue", "nudge.get_issue") > 1);
    }
    use super::*;
    #[test]
    fn source_json_and_toml_round_trip_and_reject_unknown_fields() {
        let source = Source {
            slug: "fixture".into(),
            requires_account: true,
            transport: Transport::McpHttp {
                url: "https://example.com/mcp".into(),
                bearer_env: None,
            },
        };
        let value = serde_json::to_value(&source).unwrap();
        assert_eq!(
            serde_json::from_value::<Source>(value.clone()).unwrap(),
            source
        );
        let mut unknown = value;
        unknown["credential"] = serde_json::json!("must-not-be-accepted");
        assert!(serde_json::from_value::<Source>(unknown).is_err());
        let config:Config=toml::from_str("[[sources]]\nslug='legacy'\nkind='mcp_http'\nurl='https://example.com/mcp'\nbearer_env='HOME'\n").unwrap();
        assert_eq!(config.sources.len(), 1);
    }
    #[test]
    fn rejects_embedded_secrets_and_legacy_sse() {
        for url in [
            "https://user:secret@example.com/mcp",
            "https://example.com/sse",
            "https://example.com/mcp?token=secret",
            "file:///tmp/mcp",
        ] {
            let config = Config {
                sources: vec![Source {
                    requires_account: false,
                    slug: "test".into(),
                    transport: Transport::McpHttp {
                        url: url.into(),
                        bearer_env: None,
                    },
                }],
            };
            assert!(config.validate().is_err());
        }
    }
}
