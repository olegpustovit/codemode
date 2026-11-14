pub mod approvals;
pub mod log;
use super::{auth::Principal, sources::Sources};
use crate::infrastructure::{
    postgres::{Database, Error as DbError},
    sandbox::{OUTPUT, Sandbox, Step},
};
use futures_util::{StreamExt, stream};
use serde::Serialize;
use serde_json::value::RawValue;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Semaphore, oneshot};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("gateway busy: all execution slots are in use; try again later")]
    Busy,
    #[error("Python source exceeds 64 KB")]
    Source,
    #[error("execution stopped")]
    Stopped,
    #[error(transparent)]
    Script(#[from] super::scripts::Error),
    #[error(transparent)]
    Approval(#[from] approvals::Error),
    #[error(transparent)]
    Database(#[from] DbError),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultValue {
    pub execution_id: Uuid,
    pub status: &'static str,
    pub result: Box<RawValue>,
    pub output: String,
    pub result_truncated: bool,
    pub result_dropped_bytes: usize,
    pub output_truncated: bool,
    pub output_dropped_bytes: usize,
    pub tool_calls: usize,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_url: Option<String>,
}

#[derive(Clone, Copy)]
pub struct Limits {
    pub result_bytes: usize,
    pub concurrent_executions: usize,
    pub concurrent_calls: usize,
    pub upstream_timeout: Duration,
    pub approval_lifetime: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            result_bytes: OUTPUT,
            concurrent_executions: 8,
            concurrent_calls: 16,
            upstream_timeout: Duration::from_secs(30),
            approval_lifetime: Duration::from_secs(600),
        }
    }
}

pub struct Execution<S: Sandbox> {
    db: Database,
    sources: Sources,
    sandbox: Arc<S>,
    slots: Arc<Semaphore>,
    tasks: TaskTracker,
    stop: CancellationToken,
    limits: Limits,
    approvals: approvals::Approvals,
}

enum Job {
    Start(Principal, String),
    Resume(Principal, Uuid),
}
enum Initial {
    Start(String),
    Resume(approvals::Continuation),
}
enum Outcome {
    Done(Step),
    Paused,
}

impl<S: Sandbox + Send + Sync + 'static> Execution<S> {
    pub fn new(
        db: Database,
        sources: Sources,
        sandbox: Arc<S>,
        tasks: TaskTracker,
        stop: CancellationToken,
        limits: Limits,
        snapshot_key: [u8; 32],
    ) -> Self {
        Self {
            approvals: approvals::Approvals::new(db.clone(), snapshot_key),
            db,
            sources,
            sandbox,
            slots: Arc::new(Semaphore::new(limits.concurrent_executions)),
            tasks,
            stop,
            limits,
        }
    }
    pub fn with_database(&self, db: Database, sources: Sources, stop: CancellationToken) -> Self {
        Self {
            approvals: self.approvals.with_database(db.clone()),
            db,
            sources,
            sandbox: self.sandbox.clone(),
            slots: self.slots.clone(),
            tasks: self.tasks.clone(),
            stop,
            limits: self.limits,
        }
    }
    pub fn start_sources(&self) {
        let sources = self.sources.clone();
        let stop = self.stop.clone();
        self.tasks.spawn(async move {
            sources.run(stop).await;
        });
    }
    pub fn approvals(&self) -> approvals::Approvals {
        self.approvals.clone()
    }
    pub async fn recover(&self) -> Result<(), Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("UPDATE executions SET status='failed',error='Gateway restarted during execution',finished_at=now() WHERE status='running'").execute(&mut *tx).await.map_err(DbError::from)?;
        sqlx::query!("UPDATE tool_calls c SET outcome='cancelled',error='Gateway restarted; upstream may have completed' WHERE c.outcome='running' OR (c.outcome='awaiting_approval' AND EXISTS(SELECT 1 FROM executions e WHERE e.id=c.execution_id AND e.status<>'paused'))").execute(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        self.approvals.sweep().await?;
        Ok(())
    }
    pub async fn execute(
        self: &Arc<Self>,
        principal: Principal,
        code: String,
    ) -> Result<ResultValue, Error> {
        if code.len() > 64 * 1024 {
            return Err(Error::Source);
        }
        self.submit(Job::Start(principal, code)).await
    }
    pub fn execute_script(
        self: &Arc<Self>,
        principal: Principal,
        id: Uuid,
    ) -> futures_util::future::BoxFuture<'_, Result<ResultValue, Error>> {
        // Erase this cold SQL-backed future before it enters the MCP dispatcher's state machine.
        Box::pin(async move {
            let code = super::scripts::Scripts::new(self.db.clone())
                .code_for(principal.workspace_id, id)
                .await?;
            self.execute(principal, code).await
        })
    }
    pub async fn resume(
        self: &Arc<Self>,
        principal: Principal,
        id: Uuid,
    ) -> Result<ResultValue, Error> {
        self.submit(Job::Resume(principal, id)).await
    }
    async fn submit(self: &Arc<Self>, job: Job) -> Result<ResultValue, Error> {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let cancel = self.stop.child_token();
        let guard = cancel.clone().drop_guard();
        let this = self.clone();
        let (tx, rx) = oneshot::channel();
        self.tasks.spawn(async move{
            let _permit=permit;
            let result=match job {
                Job::Start(principal,code)=>{
                    match sqlx::query_scalar!("INSERT INTO executions(token_id,code,status) VALUES($1,$2,'running') RETURNING id",principal.token_id,code).fetch_one(&this.db.0).await.map_err(DbError::from) {
                        Ok(id)=>Box::pin(this.segment(principal,id,Initial::Start(code),cancel)).await,
                        Err(error)=>Err(Error::Database(error)),
                    }
                }
                Job::Resume(principal,id)=>match this.approvals.claim(id,&principal).await {
                    Ok(approvals::Claim::Pending(count))=>Ok(this.paused(id,count)),
                    Ok(approvals::Claim::Ready(continuation))=>Box::pin(this.segment(principal,id,Initial::Resume(continuation),cancel)).await,
                    Err(error)=>Err(Error::Approval(error)),
                },
            };
            let _=tx.send(result);
        });
        let result = rx.await.map_err(|_| Error::Stopped)?;
        guard.disarm();
        result
    }
    fn paused(&self, id: Uuid, count: usize) -> ResultValue {
        ResultValue {
            execution_id: id,
            status: "paused",
            result: RawValue::NULL.to_owned(),
            output: String::new(),
            result_truncated: false,
            result_dropped_bytes: 0,
            output_truncated: false,
            output_dropped_bytes: 0,
            tool_calls: count,
            error: None,
            approval_url: Some(format!(
                "https://{}/approvals/{id}",
                self.sources.main_hostname()
            )),
        }
    }
    async fn dispatch_batch(
        &self,
        id: Uuid,
        first: usize,
        principal: &Principal,
        pending: Vec<super::broker::Prepared>,
        decisions: Option<&[(u32, bool)]>,
    ) -> Vec<(u32, crate::infrastructure::sandbox::CallResult)> {
        use super::broker::Authority;
        stream::iter(pending.into_iter().enumerate())
            .map(|(index, pending)| {
                let authority = match decisions {
                    None => Authority::Unapproved,
                    Some(decisions) => match (
                        pending.error,
                        pending.binding,
                        decisions
                            .iter()
                            .find(|(call, _)| *call == pending.call.id)
                            .map(|(_, allow)| *allow),
                    ) {
                        (Some(error), _, _) => Authority::Rejected(error),
                        (None, Some(binding), Some(true)) => Authority::Approved(binding),
                        _ => Authority::Rejected("denied by owner".into()),
                    },
                };
                self.sources.dispatch_authorized(
                    id,
                    first + index,
                    pending.call,
                    self.limits.upstream_timeout,
                    principal.workspace_id,
                    authority,
                )
            })
            .buffer_unordered(self.limits.concurrent_calls)
            .collect()
            .await
    }
    async fn segment(
        &self,
        principal: Principal,
        id: Uuid,
        initial: Initial,
        cancel: CancellationToken,
    ) -> Result<ResultValue, Error> {
        use crate::infrastructure::sandbox::Error as SandboxError;
        use sha2::Digest;
        let mut count = match &initial {
            Initial::Start(_) => 0,
            Initial::Resume(saved) => saved.frozen.count,
        };
        let code_hash = match &initial {
            Initial::Start(code) => sha2::Sha256::digest(code.as_bytes()).into(),
            Initial::Resume(saved) => saved.frozen.code_hash,
        };
        let future = async {
            let (mut run, mut step) = match initial {
                Initial::Start(code) => {
                    let allowed = self
                        .sources
                        .allowed_sources(principal.workspace_id)
                        .await
                        .map_err(|_| SandboxError::Invalid("endpoint scope unavailable".into()))?;
                    let prelude = self
                        .sources
                        .prelude_scoped(allowed.as_deref(), principal.workspace_id)
                        .await
                        .map_err(|_| SandboxError::Invalid("endpoint policy unavailable".into()))?;
                    self.sandbox
                        .start_for(principal.token_id, &code, &prelude)
                        .await?
                }
                Initial::Resume(saved) => {
                    let (run, ids) = self
                        .sandbox
                        .restore_for(principal.token_id, &saved.snapshot)
                        .await?;
                    if ids.len() != saved.frozen.pending.len()
                        || saved
                            .frozen
                            .pending
                            .iter()
                            .any(|p| !ids.contains(&p.call.id))
                    {
                        return Err(SandboxError::Snapshot);
                    }
                    let first = count
                        .checked_sub(saved.frozen.pending.len())
                        .ok_or(SandboxError::Snapshot)?;
                    let results = self
                        .dispatch_batch(
                            id,
                            first,
                            &principal,
                            saved.frozen.pending,
                            Some(&saved.decisions),
                        )
                        .await;
                    self.sandbox.resume(run, results).await?
                }
            };
            loop {
                match step {
                    Step::Done { .. } => return Ok::<_, SandboxError>(Outcome::Done(step)),
                    Step::Calls(calls) => {
                        if calls.len() > 100usize.saturating_sub(count) {
                            return Err(SandboxError::Invalid(
                                "execution exceeds its 100 tool-call limit".into(),
                            ));
                        }
                        if calls
                            .iter()
                            .map(|call| call.args.get().len() + call.path.len())
                            .sum::<usize>()
                            > 8 * 1024 * 1024
                        {
                            return Err(SandboxError::Invalid(
                                "pending tool arguments exceed 8 MiB".into(),
                            ));
                        }
                        let first = count;
                        count += calls.len();
                        let pending = self
                            .sources
                            .prepare(calls, principal.workspace_id)
                            .await
                            .map_err(|_| {
                                SandboxError::Invalid(
                                    "policy unavailable; no calls in this batch were sent".into(),
                                )
                            })?;
                        if pending.iter().any(|call| call.needs_approval) {
                            let snapshot = self.sandbox.snapshot(run).await?;
                            let expires = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .ok()
                                .and_then(|v| i64::try_from(v.as_secs()).ok())
                                .and_then(|v| {
                                    i64::try_from(self.limits.approval_lifetime.as_secs())
                                        .ok()
                                        .and_then(|lifetime| v.checked_add(lifetime))
                                })
                                .ok_or(SandboxError::Snapshot)?;
                            let frozen = approvals::Frozen {
                                execution_id: id,
                                pause_id: Uuid::new_v4(),
                                token_id: principal.token_id,
                                audience: principal.audience.clone(),
                                expires,
                                code_hash,
                                count,
                                pending,
                            };
                            self.approvals
                                .save(frozen, snapshot)
                                .await
                                .map_err(|error| SandboxError::Invalid(error.to_string()))?;
                            return Ok(Outcome::Paused);
                        }
                        let results = self
                            .dispatch_batch(id, first, &principal, pending, None)
                            .await;
                        (run, step) = self.sandbox.resume(run, results).await?;
                    }
                }
            }
        };
        let (status, error, done) = tokio::select! {biased;
            _=cancel.cancelled()=>("failed",Some("execution cancelled; calls already sent upstream may have completed".into()),None),
            outcome=tokio::time::timeout(Duration::from_secs(60),future)=>match outcome {
                Err(_)=>("timed_out",Some("execution exceeded its 60 second wall deadline; worker terminated".into()),None),
                Ok(Err(error))=>("failed",Some(error.agent_message()),None),
                Ok(Ok(Outcome::Paused))=>return Ok(self.paused(id,count)),
                Ok(Ok(Outcome::Done(step)))=>("completed",None,Some(step)),
            }
        };
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("UPDATE tool_calls SET outcome='cancelled',error='Execution ended while call was in flight; upstream may have completed' WHERE execution_id=$1 AND outcome IN ('running','awaiting_approval') AND NOT EXISTS(SELECT 1 FROM paused_runs p WHERE p.execution_id=$1)",id).execute(&mut *tx).await.map_err(DbError::from)?;
        sqlx::query!("UPDATE executions SET status=$2,error=$3,tool_calls=$4,finished_at=now() WHERE id=$1 AND status='running'",id,status,error,count as i32).execute(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        let (result, output, output_dropped_bytes) = match done {
            Some(Step::Done {
                result,
                output,
                output_dropped_bytes,
            }) => (result, output, output_dropped_bytes),
            _ => (RawValue::NULL.to_owned(), String::new(), 0),
        };
        let (result, result_dropped_bytes) = truncate_result(result, self.limits.result_bytes);
        Ok(ResultValue {
            execution_id: id,
            status,
            result,
            output,
            result_truncated: result_dropped_bytes > 0,
            result_dropped_bytes,
            output_truncated: output_dropped_bytes > 0,
            output_dropped_bytes,
            tool_calls: count,
            error,
            approval_url: None,
        })
    }
}

fn truncate_result(value: Box<RawValue>, budget: usize) -> (Box<RawValue>, usize) {
    if value.get().len() <= budget {
        return (value, 0);
    }
    // Truncated JSON becomes a string preview, so the MCP result always remains valid JSON.
    // Escaping quotes, backslashes and controls also consumes the response budget.
    let mut remaining = budget.saturating_sub(2);
    let mut length = 0;
    for c in value.get().chars() {
        let encoded = match c {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}' => 2,
            c if c <= '\u{1f}' => 6,
            c => c.len_utf8(),
        };
        if encoded > remaining {
            break;
        }
        remaining -= encoded;
        length += c.len_utf8();
    }
    let preview = &value.get()[..length];
    let dropped = value.get().len() - preview.len();
    (
        serde_json::value::to_raw_value(preview).unwrap_or_else(|_| RawValue::NULL.to_owned()),
        dropped,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncated_result_is_valid_utf8_json_with_exact_drop_count() {
        let raw = serde_json::value::to_raw_value(&"é".repeat(OUTPUT)).unwrap();
        let original = raw.get().len();
        let (preview, dropped) = truncate_result(raw, OUTPUT);
        let decoded: String = serde_json::from_str(preview.get()).unwrap();
        assert_eq!(decoded.len() + dropped, original);
        assert!(preview.get().len() <= OUTPUT + 8);
    }

    #[test]
    fn truncated_result_budget_includes_json_escaping() {
        for text in [
            "\"".repeat(OUTPUT),
            "\\".repeat(OUTPUT),
            "\n".repeat(OUTPUT),
        ] {
            let raw = serde_json::value::to_raw_value(&text).unwrap();
            let original = raw.get().len();
            let (preview, dropped) = truncate_result(raw, OUTPUT);
            let decoded: String = serde_json::from_str(preview.get()).unwrap();
            assert!(preview.get().len() <= OUTPUT);
            assert_eq!(decoded.len() + dropped, original);
        }
    }
}
