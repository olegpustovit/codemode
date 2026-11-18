mod http;
pub(crate) const SEARCH_PATH: &str = "@codemode/search";
pub(crate) const SAVE_SCRIPT_PATH: &str = "@codemode/save_script";
pub(crate) const MEMO_PATH: &str = "@codemode/memo";
pub(crate) const UNKNOWN_PATH: &str = "@codemode/unknown";
use super::{
    accounts::Accounts,
    catalogue::{Catalogue, Definition},
    policies::{Action, Policies},
    workspaces::Workspaces,
};
use crate::infrastructure::{
    postgres::Database,
    sandbox::{CallFailure, CallResult, PendingCall},
};
use rmcp::{
    model::CallToolRequestParams,
    service::{Peer, RoleClient},
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, value::RawValue};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
enum CallError {
    #[error("{0}")]
    Blocked(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Upstream(String),
    #[error("upstream call timed out after {0} seconds")]
    Timeout(u64),
}

#[derive(Clone)]
pub(crate) enum Credentials {
    Http,
    Stdio(Arc<Vec<SecretString>>),
}

#[derive(Clone)]
pub struct Tool {
    pub(crate) source_id: Uuid,
    pub revision: i64,
    pub definition: Definition,
    pub(crate) validator: Arc<jsonschema::Validator>,
    pub(crate) omit_null: Arc<std::collections::BTreeSet<String>>,
    pub(crate) target: Target,
    pub(crate) observed: Arc<AtomicBool>,
}
#[derive(Clone)]
pub(crate) enum Target {
    Mcp {
        peer: Peer<RoleClient>,
        replies: crate::infrastructure::upstream::Replies,
        credentials: Credentials,
    },
    Graphql {
        kind: super::graphql::Kind,
        client: reqwest::Client,
        url: String,
        required: bool,
    },
    OpenApi {
        operation: Arc<super::openapi::Operation>,
        client: reqwest::Client,
        server: String,
        required: bool,
    },
}

#[derive(Clone)]
pub(crate) struct Broker {
    db: Database,
    accounts: Accounts,
    origin: String,
    workspaces: Workspaces,
    policies: Policies,
}
impl Broker {
    pub fn new(
        db: Database,
        accounts: Accounts,
        endpoints: Workspaces,
        policies: Policies,
    ) -> Self {
        Self {
            origin: format!("https://{}", endpoints.main_hostname()),
            workspaces: endpoints,
            db,
            accounts,
            policies,
        }
    }
    pub fn http(
        &self,
        source: Uuid,
        required: bool,
        replies: crate::infrastructure::upstream::Replies,
    ) -> Result<http::Client, reqwest::Error> {
        http::Client::new(
            self.accounts.clone(),
            source,
            required,
            self.account_link(source),
            replies,
        )
    }
    pub async fn environment(
        &self,
        source: Uuid,
    ) -> Result<std::collections::BTreeMap<String, SecretString>, super::accounts::Error> {
        self.accounts.environment(source).await
    }
    pub(crate) fn account_link(&self, source: Uuid) -> String {
        format!("{}/sources/{source}/accounts", self.origin)
    }
    async fn search(
        &self,
        args: Value,
        workspace: Uuid,
        execution: Uuid,
    ) -> Result<Box<RawValue>, CallError> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            query: String,
        }
        let input: Input = serde_json::from_value(args)
            .map_err(|_| CallError::Invalid("tools.search requires query: str".into()))?;
        let allowed = self
            .workspaces
            .sources(workspace)
            .await
            .map_err(|_| CallError::Blocked("workspace scope unavailable".into()))?;
        let mut page = super::catalogue::Catalogue::new(self.db.clone())
            .search_scoped(
                &input.query,
                false,
                None,
                allowed.as_deref(),
                Some(workspace),
            )
            .await
            .map_err(|error| CallError::Invalid(error.to_string()))?;
        let admin = sqlx::query_scalar!("SELECT t.admin FROM access_tokens t JOIN executions e ON e.token_id=t.id WHERE e.id=$1 AND t.workspace_id=$2 AND t.revoked_at IS NULL",execution,workspace)
            .fetch_optional(&self.db.0).await.map_err(|_| CallError::Upstream("administration lookup unavailable".into()))?
            .unwrap_or(false);
        super::administration::discover(&input.query, admin, &mut page.tools);
        serde_json::value::to_raw_value(if page.tools.is_empty() {
            &page.suggestions
        } else {
            &page.tools
        })
        .map_err(|_| CallError::Upstream("could not encode search results".into()))
    }

    async fn observe_shape(&self, tool: &Tool, reply: &RawValue) {
        let generic = tool.definition.output_schema.as_ref().is_none_or(|schema| {
            schema
                .get("properties")
                .and_then(Value::as_object)
                .is_none_or(|keys| keys.is_empty())
                && schema.get("$ref").is_none()
                && schema.get("anyOf").is_none()
        });
        if !generic || tool.observed.load(Ordering::Relaxed) {
            return;
        }
        if let Some(shape) = super::execution::shape::learned(reply.get())
            && !tool.observed.swap(true, Ordering::Relaxed)
            && let Err(error) = Catalogue::new(self.db.clone())
                .remember_shape(tool.source_id, tool.revision, &tool.definition.path, &shape)
                .await
        {
            tool.observed.store(false, Ordering::Relaxed);
            tracing::warn!(?error, "could not save observed tool shape");
        }
    }

    async fn save_script(
        &self,
        execution: Uuid,
        workspace: Uuid,
        args: Value,
    ) -> Result<Box<RawValue>, CallError> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Save {
            name: String,
            description: String,
            code: String,
            scope: super::scripts::Scope,
        }
        let input: Save = serde_json::from_value(args).map_err(|_| {
            CallError::Invalid(
                "save_script requires name, description, code, and scope workspace or global"
                    .into(),
            )
        })?;
        let content = super::scripts::Content {
            name: input.name,
            description: input.description,
            code: input.code,
        };
        let id = super::scripts::Scripts::new(self.db.clone())
            .save_from_execution(workspace, execution, input.scope, content)
            .await
            .map_err(|error| match error {
                super::scripts::Error::Permission => CallError::Blocked(error.to_string()),
                super::scripts::Error::Database(_) => {
                    CallError::Upstream("script storage unavailable".into())
                }
                error => CallError::Invalid(error.to_string()),
            })?;
        serde_json::value::to_raw_value(&serde_json::json!({"scriptId":id}))
            .map_err(|_| CallError::Upstream("could not encode saved script".into()))
    }

    pub async fn dispatch(
        &self,
        sources: &super::sources::Sources,
        execution: Uuid,
        seq: usize,
        call: PendingCall,
        tool: Result<Tool, String>,
        timeout: Duration,
        workspace: Uuid,
    ) -> (u32, CallResult) {
        let started = Instant::now();
        let args: Result<Value, _> = serde_json::from_str(call.args.get());
        let redacted = if call.path == MEMO_PATH {
            let op = match args
                .as_ref()
                .ok()
                .and_then(|args| args.get("op"))
                .and_then(Value::as_str)
            {
                Some("get") => "get",
                Some("set") => "set",
                _ => "invalid",
            };
            serde_json::json!({"op":op,"contents":"[REDACTED]"})
        } else if call.path.starts_with(super::administration::PREFIX) {
            serde_json::json!({"arguments":"[REDACTED]"})
        } else if call.path == SAVE_SCRIPT_PATH {
            serde_json::json!({"script":"[REDACTED]"})
        } else {
            args.as_ref().map(redact).unwrap_or(Value::Null)
        };
        let audit=sqlx::query_scalar!("INSERT INTO tool_calls(execution_id,seq,path,args_redacted,outcome) VALUES($1,$2,$3,$4,'running') RETURNING id",execution,seq as i32,call.path,redacted).fetch_one(&self.db.0).await;
        let Ok(id) = audit else {
            return (
                call.id,
                Err("audit unavailable; tool was not called".into()),
            );
        };
        let result = match args {
            Ok(_) if call.path == UNKNOWN_PATH => {
                Err(CallError::Invalid(tool.err().unwrap_or_else(|| {
                    "unknown-tool lookup unavailable".into()
                })))
            }
            Ok(args) if call.path == SEARCH_PATH => self.search(args, workspace, execution).await,
            Ok(args) if call.path.starts_with(super::administration::PREFIX) => {
                super::administration::run(
                    &self.db,
                    sources,
                    &self.workspaces,
                    &self.accounts,
                    execution,
                    workspace,
                    &call.path[super::administration::PREFIX.len()..],
                    args,
                    timeout,
                )
                .await
                .map_err(|error| match error {
                    super::administration::Error::Permission => {
                        CallError::Blocked(error.to_string())
                    }
                    super::administration::Error::Input(_) => CallError::Invalid(error.to_string()),
                    super::administration::Error::Timeout => CallError::Timeout(timeout.as_secs()),
                    error => CallError::Upstream(error.to_string()),
                })
            }
            Ok(args) if call.path == SAVE_SCRIPT_PATH => {
                match tokio::time::timeout(timeout, self.save_script(execution, workspace, args))
                    .await
                {
                    Ok(result) => result,
                    Err(_) => Err(CallError::Timeout(timeout.as_secs())),
                }
            }
            Ok(args) if call.path == MEMO_PATH => {
                match serde_json::from_value::<super::agent_sessions::memo::Request>(args) {
                    Ok(request) => super::agent_sessions::AgentSessions::new(self.db.clone())
                        .memo(execution, workspace, request)
                        .await
                        .map_err(|error| match error {
                            super::agent_sessions::Error::NotFound => {
                                CallError::Blocked("agent session memo is unavailable".into())
                            }
                            super::agent_sessions::Error::MemoLimit
                            | super::agent_sessions::Error::MemoSensitive => {
                                CallError::Invalid(error.to_string())
                            }
                            _ => {
                                CallError::Upstream("agent session memo storage unavailable".into())
                            }
                        })
                        .and_then(|value| {
                            serde_json::value::to_raw_value(&value).map_err(|_| {
                                CallError::Upstream("could not encode memo result".into())
                            })
                        }),
                    Err(_) => Err(CallError::Invalid(
                        "memo requires op=get/set, a string key, and a JSON value for set".into(),
                    )),
                }
            }
            Ok(mut args) => match tool {
                Ok(tool) => {
                    strip_optional_nulls(&mut args, &tool.omit_null);
                    let revision = sqlx::query_scalar!(
                        "SELECT s.revision FROM sources s JOIN endpoints e ON e.id=$2 WHERE s.id=$1 AND (e.all_sources OR EXISTS(SELECT 1 FROM endpoint_sources es WHERE es.source_id=s.id AND es.endpoint_id=e.id))",
                        tool.source_id,workspace
                    )
                    .fetch_optional(&self.db.0)
                    .await;
                    match revision {
                        Ok(Some(revision)) if revision == tool.revision => {
                            if let Err(error) = tool.validate(&args) {
                                Err(CallError::Invalid(error))
                            } else {
                                match self
                                    .policies
                                    .effective(
                                        workspace,
                                        tool.source_id,
                                        &tool.definition.upstream_name,
                                    )
                                    .await
                                {
                                    Ok(Action::Block) => {
                                        Err(CallError::Blocked(unavailable(&call.path, workspace)))
                                    }
                                    Ok(Action::Allow) => {
                                        let reply = tool
                                            .call(
                                                args,
                                                timeout,
                                                &self.account_link(tool.source_id),
                                                &self.accounts,
                                            )
                                            .await;
                                        if let Ok(value) = &reply {
                                            self.observe_shape(&tool, value).await;
                                        }
                                        reply
                                    }
                                    Err(_) => Err(CallError::Blocked(
                                        "policy unavailable; tool was not called".into(),
                                    )),
                                }
                            }
                        }
                        Ok(Some(_)) => Err(CallError::Upstream(format!(
                            "source changed; search again after synchronization; account: {}",
                            self.account_link(tool.source_id)
                        ))),
                        Ok(None) => Err(CallError::Blocked(unavailable(&call.path, workspace))),
                        Err(_) => Err(CallError::Upstream(
                            "source state unavailable; tool was not called".into(),
                        )),
                    }
                }
                Err(error) => Err(CallError::Upstream(error)),
            },
            Err(_) => Err(CallError::Invalid("tool arguments are invalid JSON".into())),
        };
        let outcome = match &result {
            Ok(_) => "ok",
            Err(CallError::Invalid(_)) => "invalid_args",
            Err(CallError::Blocked(_)) => "blocked",
            Err(CallError::Timeout(_)) => "timeout",
            Err(CallError::Upstream(_)) => "tool_error",
        };
        let error = result.as_ref().err().map(ToString::to_string);
        let preview = if call.path == MEMO_PATH {
            None
        } else {
            result
                .as_ref()
                .ok()
                .map(|value| super::execution::log::preview(value.get(), 2048))
        };
        let elapsed = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
        if sqlx::query!(
            "UPDATE tool_calls SET outcome=$2,error=$3,duration_ms=$4,preview=$5 WHERE id=$1",
            id,
            outcome,
            error,
            elapsed,
            preview
        )
        .execute(&self.db.0)
        .await
        .is_err()
        {
            return (call.id,Err("tool returned but its audit could not be finalized; do not automatically retry".into()));
        }
        (
            call.id,
            result.map_err(|error| match error {
                CallError::Blocked(message) => CallFailure::Permission(message),
                other => CallFailure::from(other.to_string()),
            }),
        )
    }
}

pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        if secret_key(k) {
                            Value::String("[REDACTED]".into())
                        } else {
                            redact(v)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact).collect()),
        Value::String(s) if s.len() > 1024 => {
            Value::String(format!("{}… [{} bytes]", prefix(s, 128), s.len()))
        }
        _ => value.clone(),
    }
}

pub(crate) fn unavailable(path: &str, workspace: Uuid) -> String {
    format!("tool {path} is blocked in workspace {workspace}; use search to find allowed tools")
}

pub(crate) fn secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace(['-', '_'], "");
    [
        "token",
        "secret",
        "password",
        "authorization",
        "apikey",
        "cookie",
    ]
    .iter()
    .any(|s| key.contains(s))
}

pub fn redact_message(message: &str, args: &Value) -> String {
    fn visit(value: &Value, secret: bool, message: &mut String) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    visit(value, secret || secret_key(key), message);
                }
            }
            Value::Array(values) => {
                for value in values {
                    visit(value, secret, message);
                }
            }
            Value::String(value) if secret && !value.is_empty() => {
                *message = message.replace(value, "[REDACTED]")
            }
            _ => {}
        }
    }
    let mut message = message.to_owned();
    visit(args, false, &mut message);
    prefix(&message, 4096).to_owned()
}

fn prefix(value: &str, max: usize) -> &str {
    let mut n = value.len().min(max);
    while !value.is_char_boundary(n) {
        n -= 1;
    }
    &value[..n]
}

// Compile property validators against the original root so local references keep their meaning.
pub(crate) fn optional_nonnullable(
    schema: &Value,
) -> Result<std::collections::BTreeSet<String>, jsonschema::ValidationError<'static>> {
    let validators = jsonschema::options().offline().build_map(schema)?;
    let required = schema.get("required").and_then(Value::as_array);
    Ok(schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|p| p.keys())
        .filter(|key| !required.is_some_and(|r| r.iter().any(|v| v.as_str() == Some(key.as_str()))))
        .filter(|key| {
            validators
                .get(&format!(
                    "#/properties/{}",
                    key.replace('~', "~0").replace('/', "~1")
                ))
                .is_some_and(|v| !v.is_valid(&Value::Null))
        })
        .cloned()
        .collect())
}

fn strip_optional_nulls(args: &mut Value, omit: &std::collections::BTreeSet<String>) {
    if let Some(args) = args.as_object_mut() {
        args.retain(|key, value| !value.is_null() || !omit.contains(key));
    }
}

impl Tool {
    pub(crate) fn validate(&self, args: &Value) -> Result<(), String> {
        if let Some(error) = self.validator.iter_errors(args).next() {
            // Never render the validation error itself: it includes the supplied secret value.
            use jsonschema::error::{TypeKind, ValidationErrorKind};
            let expected = match error.kind() {
                ValidationErrorKind::Type {
                    kind: TypeKind::Single(kind),
                } => format!("expected {kind}"),
                ValidationErrorKind::Type {
                    kind: TypeKind::Multiple(kinds),
                } => format!(
                    "expected {}",
                    kinds
                        .iter()
                        .map(|kind| kind.to_string())
                        .collect::<Vec<_>>()
                        .join(" or ")
                ),
                _ => format!("constraint {}", error.schema_path()),
            };
            return Err(format!(
                "{}: invalid argument at {} ({expected})\n  signature: {}",
                self.definition.path,
                error.instance_path(),
                super::catalogue::stubs::generate(
                    &self.definition.path,
                    "",
                    &self.definition.input_schema,
                    None
                )
                .brief
            ));
        }
        if let Target::Graphql { kind, .. } = &self.target {
            kind.document(args)?;
        }
        Ok(())
    }
    async fn call(
        &self,
        args: Value,
        timeout: Duration,
        link: &str,
        accounts: &Accounts,
    ) -> Result<Box<RawValue>, CallError> {
        if let Target::Graphql {
            kind,
            client,
            url,
            required,
        } = &self.target
        {
            let document = kind.document(&args).map_err(CallError::Invalid)?;
            let mut body = serde_json::Map::new();
            body.insert("query".into(), Value::String(document));
            for key in ["variables", "operationName"] {
                if let Some(value) = args.get(key).filter(|v| !v.is_null()) {
                    body.insert(key.into(), value.clone());
                }
            }
            let call = async {
                let credential = accounts.rest(self.source_id, url).await.map_err(|_| {
                    CallError::Upstream(format!("linked credential unavailable; link it at {link}"))
                })?;
                if *required && credential.is_none() {
                    return Err(CallError::Upstream(format!(
                        "account is not linked; link it at {link}"
                    )));
                }
                let mut request = client
                    .post(url)
                    .header(
                        reqwest::header::ACCEPT,
                        "application/graphql-response+json, application/json",
                    )
                    .json(&body)
                    .build()
                    .map_err(|_| {
                        CallError::Invalid("GraphQL request could not be encoded".into())
                    })?;
                if let Some(credential) = &credential {
                    credential.apply(&mut request).map_err(|_| {
                        CallError::Upstream(format!(
                            "linked credential unavailable; link it at {link}"
                        ))
                    })?;
                }
                let (bytes, status) =
                    super::graphql::call(client, request)
                        .await
                        .map_err(|error| {
                            CallError::Upstream(format!("{}: {error}", self.definition.path))
                        })?;
                super::graphql::result(&bytes, status).map_err(|error| {
                    let message = error.to_string();
                    let message = credential
                        .as_ref()
                        .map_or_else(|| message.clone(), |c| c.redact(&message));
                    CallError::Upstream(format!(
                        "{}: {}",
                        self.definition.path,
                        redact_message(&message, &args)
                    ))
                })
            };
            return tokio::time::timeout(timeout, call)
                .await
                .map_err(|_| CallError::Timeout(timeout.as_secs()))?;
        }
        let arguments = args
            .as_object()
            .cloned()
            .ok_or_else(|| CallError::Invalid("tool arguments must be an object".into()))?;
        let Target::Mcp {
            peer,
            replies,
            credentials,
        } = &self.target
        else {
            let Target::OpenApi {
                operation,
                client,
                server,
                required,
            } = &self.target
            else {
                return Err(CallError::Upstream("invalid tool target".into()));
            };
            let call = async {
                let mut request = operation
                    .request(client, &arguments)
                    .map_err(|error| CallError::Invalid(error.to_string()))?;
                let credential = accounts.rest(self.source_id, server).await.map_err(|_| {
                    CallError::Upstream(format!("linked credential unavailable; link it at {link}"))
                })?;
                if *required && credential.is_none() {
                    return Err(CallError::Upstream(format!(
                        "account is not linked; link it at {link}"
                    )));
                }
                if let Some(credential) = credential {
                    credential.apply(&mut request).map_err(|_| {
                        CallError::Upstream(format!(
                            "linked credential unavailable; link it at {link}"
                        ))
                    })?;
                }
                operation.call(client, request).await.map_err(|error| {
                    CallError::Upstream(format!(
                        "{}: {error}; account: {link}",
                        self.definition.path
                    ))
                })
            };
            return tokio::time::timeout(timeout, call)
                .await
                .map_err(|_| CallError::Timeout(timeout.as_secs()))?;
        };
        let result = tokio::time::timeout(
            timeout,
            peer.call_tool(
                CallToolRequestParams::new(self.definition.upstream_name.clone())
                    .with_arguments(arguments),
            ),
        )
        .await
        .map_err(|_| CallError::Timeout(timeout.as_secs()))?
        .map_err(|_| {
            CallError::Upstream(format!(
                "upstream connection failed; check the account at {link}"
            ))
        })?;
        replies
            .take(result)
            .map_err(CallError::Upstream)?
            .result()
            .map_err(|message| {
                let mut message = redact_message(&message, &args);
                if let Credentials::Stdio(secrets) = credentials {
                    for secret in secrets.iter().filter(|s| !s.expose_secret().is_empty()) {
                        message = message.replace(secret.expose_secret(), "[REDACTED]");
                    }
                }
                CallError::Upstream(format!(
                    "{}: upstream tool failed: {message}",
                    self.definition.path
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn optional_nulls_preserve_required_nullable_nested_and_referenced_values() {
        let schema = json!({"type":"object", "$defs":{"text":{"type":"string"}},
            "required":["required"], "properties":{
                "cursor":{"$ref":"#/$defs/text"}, "required":{"type":"string"},
                "nullable":{"anyOf":[{"type":"string"},{"type":"null"}]},
                "nested":{"type":"object"}, "a/b~c":{"type":"integer"}}});
        let omit = optional_nonnullable(&schema).unwrap();
        let mut args = json!({"cursor":null,"required":null,"nullable":null,
            "nested":{"cursor":null},"unknown":null,"a/b~c":null});
        strip_optional_nulls(&mut args, &omit);
        assert_eq!(
            args,
            json!({"required":null,"nullable":null,"nested":{"cursor":null},"unknown":null})
        );
        assert!(!jsonschema::validator_for(&schema).unwrap().is_valid(&args));
        args["required"] = json!("present");
        assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&args));
    }

    #[test]
    fn redacts_nested_secrets_in_arguments_and_error_messages() {
        let args = json!({"nested":[{"api-key":"sekret","name":"fine"}],"long":"é".repeat(2000)});
        assert_eq!(redact(&args)["nested"][0]["api-key"], "[REDACTED]");
        assert_eq!(
            redact_message("failed with sekret", &args),
            "failed with [REDACTED]"
        );
        assert!(redact(&args)["long"].as_str().unwrap().len() < 160);
    }
}
