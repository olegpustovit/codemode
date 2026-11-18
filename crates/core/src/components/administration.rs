use super::{
    accounts::Accounts,
    catalogue::Match,
    execution::log::Log,
    integrations,
    sources::{SourceInput, Sources, Transport},
    workspaces::Workspaces,
};
use crate::infrastructure::postgres::Database;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, value::RawValue};
use std::time::Duration;
use uuid::Uuid;

pub(crate) const PREFIX: &str = "@codemode/admin/";

const TOOLS: &[(&str, &str, &str)] = &[
    (
        "list_workspaces",
        "()",
        "List your workspaces and their source membership.",
    ),
    (
        "save_workspace",
        "(name: str, sourceIds: list, id: str | None = None)",
        "Create or update a workspace.",
    ),
    (
        "delete_workspace",
        "(id: str)",
        "Delete a custom workspace and revoke its agent access.",
    ),
    (
        "list_integrations",
        "()",
        "List your integration connections and link state.",
    ),
    (
        "list_sources",
        "()",
        "List connected sources without credentials or configuration.",
    ),
    (
        "save_source",
        "(source: dict, id: str | None = None)",
        "Create or update a remote tool source.",
    ),
    (
        "delete_source",
        "(id: str)",
        "Delete a source and its linked credentials.",
    ),
    (
        "sync_source",
        "(id: str)",
        "Request synchronization for a source.",
    ),
    (
        "list_accounts",
        "()",
        "List linked account metadata and expiry.",
    ),
    (
        "source_connection_url",
        "(id: str)",
        "Get the browser URL for linking a source account.",
    ),
];

pub fn discover(query: &str, admin: bool, matches: &mut Vec<Match>) {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return;
    }
    let mut add = |path: &str, signature: String, description: &str| {
        let text = format!("{signature} {description} {path}").to_ascii_lowercase();
        if query.split(" or ").any(|group| {
            !group.is_empty() && group.split_whitespace().all(|term| text.contains(term))
        }) {
            matches.push(Match {
                suggested: false,
                blocked: false,
                path: path.into(),
                description: description.into(),
                signature,
            });
        }
    };
    add(
        "@codemode/save_script",
        "codemode.save_script(name: str, description: str, code: str, scope: str = 'workspace')"
            .into(),
        "Save reusable Python and return its scriptId.",
    );
    if admin {
        for (name, args, description) in TOOLS {
            add(
                &format!("{PREFIX}{name}"),
                format!("codemode.admin.{name}{args}"),
                description,
            );
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("administration access required")]
    Permission,
    #[error("invalid administration arguments: {0}")]
    Input(String),
    #[error("{0}")]
    Operation(String),
    #[error("administration operation timed out; inspect current state before retrying")]
    Timeout,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: Uuid,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveWorkspace {
    id: Option<Uuid>,
    name: String,
    source_ids: Vec<Uuid>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveSource {
    id: Option<Uuid>,
    source: SourceInput,
}

fn input<T: DeserializeOwned>(args: Value) -> Result<T, Error> {
    serde_json::from_value(args).map_err(|error| Error::Input(error.to_string()))
}
fn encode<T: Serialize, E: std::fmt::Display>(
    result: Result<T, E>,
) -> Result<Box<RawValue>, Error> {
    let result = result.map_err(|error| Error::Operation(error.to_string()))?;
    serde_json::value::to_raw_value(&result)
        .map_err(|_| Error::Operation("could not encode administration result".into()))
}

pub(crate) async fn run(
    db: &Database,
    sources: &Sources,
    workspaces: &Workspaces,
    accounts: &Accounts,
    execution: Uuid,
    workspace: Uuid,
    tool: &str,
    args: Value,
    timeout: Duration,
) -> Result<Box<RawValue>, Error> {
    let actor = sqlx::query!("SELECT t.id,t.admin FROM executions e JOIN access_tokens t ON t.id=e.token_id WHERE e.id=$1 AND t.workspace_id=$2 AND t.revoked_at IS NULL",execution,workspace)
        .fetch_optional(&db.0).await.map_err(|_| Error::Operation("administration lookup unavailable".into()))?
        .ok_or(Error::Permission)?;
    let log = Log::new(db.clone());
    let id = log
        .start_administration(
            actor.id,
            tool,
            serde_json::json!({"arguments":"[REDACTED]"}),
        )
        .await
        .map_err(|_| Error::Operation("audit unavailable; operation was not performed".into()))?;
    let result = if actor.admin {
        match tokio::time::timeout(
            timeout,
            operation(sources, workspaces, accounts, tool, args),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(Error::Timeout),
        }
    } else {
        Err(Error::Permission)
    };
    log.finish_administration(id, result.is_ok()).await.map_err(|_| {
        Error::Operation("operation finished but its audit could not be finalized; inspect current state before retrying".into())
    })?;
    result
}

async fn operation(
    sources: &Sources,
    workspaces: &Workspaces,
    accounts: &Accounts,
    tool: &str,
    args: Value,
) -> Result<Box<RawValue>, Error> {
    match tool {
        "list_workspaces" => encode(workspaces.list().await),
        "save_workspace" => {
            let input: SaveWorkspace = input(args)?;
            encode(if let Some(id) = input.id {
                workspaces
                    .update(id, &input.name, &input.source_ids)
                    .await
                    .map(|()| id)
            } else {
                workspaces.create(&input.name, &input.source_ids).await
            })
        }
        "delete_workspace" => encode(workspaces.remove(input::<Id>(args)?.id).await),
        "list_integrations" => encode(integrations::list(sources).await),
        "list_sources" => encode(sources.list().await.map(|rows| {
            rows.into_iter()
                .map(|row| {
                    let kind = match row.source.transport {
                        Transport::McpHttp { .. } => "mcp_http",
                        Transport::McpStdio { .. } => "mcp_stdio",
                        Transport::OpenApi { .. } => "openapi",
                        Transport::Graphql { .. } => "graphql",
                    };
                    serde_json::json!({
                        "id":row.id,"name":row.name,"icon":row.icon,"slug":row.source.slug,
                        "kind":kind,"status":row.status,"toolCount":row.tool_count,
                        "requiresAccount":row.source.requires_account
                    })
                })
                .collect::<Vec<_>>()
        })),
        "save_source" => {
            let input: SaveSource = input(args)?;
            super::sources::user_source(&input.source.source)
                .map_err(|error| Error::Input(error.into()))?;
            encode(sources.save_input(input.id, input.source).await)
        }
        "delete_source" => encode(sources.remove(input::<Id>(args)?.id).await),
        "sync_source" => encode(sources.resync(input::<Id>(args)?.id).await),
        "list_accounts" => encode(accounts.list().await),
        "source_connection_url" => {
            let id = input::<Id>(args)?.id;
            encode(sources.get(id).await.and_then(|row| {
                row.ok_or(super::sources::Error::NotFound).map(|_| {
                    serde_json::json!({"url":format!("https://{}/sources/{id}/accounts",workspaces.main_hostname())})
                })
            }))
        }
        _ => Err(Error::Input("unknown administration tool".into())),
    }
}
