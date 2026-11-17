pub mod log;
pub(crate) mod title;
use super::{auth::Principal, sources::Sources};
use crate::infrastructure::{
    postgres::{Database, Error as DbError},
    sandbox::{Capture, OUTPUT, Sandbox, Step},
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
    #[error(transparent)]
    Access(#[from] super::access::Error),
    #[error("gateway busy: all execution slots are in use; try again later")]
    Busy,
    #[error("Python source exceeds 64 KB")]
    Source,
    #[error("execution stopped")]
    Stopped,
    #[error(transparent)]
    Session(#[from] super::agent_sessions::Error),
    #[error(transparent)]
    Script(#[from] super::scripts::Error),
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
}

#[derive(Clone, Copy)]
pub struct Limits {
    pub result_bytes: usize,
    pub concurrent_executions: usize,
    pub concurrent_calls: usize,
    pub upstream_timeout: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            result_bytes: OUTPUT,
            concurrent_executions: 8,
            concurrent_calls: 16,
            upstream_timeout: Duration::from_secs(30),
        }
    }
}

pub struct Execution<S: Sandbox> {
    access: super::access::Access,
    db: Database,
    sources: Sources,
    sandbox: Arc<S>,
    slots: Arc<Semaphore>,
    tasks: TaskTracker,
    stop: CancellationToken,
    limits: Limits,
}

impl<S: Sandbox + Send + Sync + 'static> Execution<S> {
    pub fn new(
        db: Database,
        sources: Sources,
        sandbox: Arc<S>,
        tasks: TaskTracker,
        stop: CancellationToken,
        limits: Limits,
    ) -> Self {
        Self {
            access: Default::default(),
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
            access: self.access.clone(),
            db,
            sources,
            sandbox: self.sandbox.clone(),
            slots: self.slots.clone(),
            tasks: self.tasks.clone(),
            stop,
            limits: self.limits,
        }
    }
    pub fn with_access(mut self, access: super::access::Access) -> Self {
        self.access = access;
        self
    }
    pub fn start_sources(&self) {
        let sources = self.sources.clone();
        let stop = self.stop.clone();
        self.tasks.spawn(async move {
            sources.run(stop).await;
        });
    }
    pub async fn recover(&self) -> Result<(), Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("UPDATE executions SET status='failed',error='Gateway restarted during execution',finished_at=now() WHERE status='running'").execute(&mut *tx).await.map_err(DbError::from)?;
        sqlx::query!("UPDATE tool_calls SET outcome='cancelled',error='Gateway restarted; upstream may have completed' WHERE outcome='running'").execute(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
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
        self.submit(principal, code).await
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
    async fn submit(
        self: &Arc<Self>,
        principal: Principal,
        code: String,
    ) -> Result<ResultValue, Error> {
        let mut lease = self.access.watch(principal.tenant_id).await?;
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let cancel = self.stop.child_token();
        let guard = cancel.clone().drop_guard();
        let this = self.clone();
        let (tx, rx) = oneshot::channel();
        self.tasks.spawn(async move {
            let _permit = permit;
            let result = match super::agent_sessions::AgentSessions::new(this.db.clone())
                .start(&principal, &code)
                .await
            {
                Ok(id) => {
                    let future = this.segment(principal, id, code, cancel.clone());
                    tokio::pin!(future);
                    tokio::select! { biased;
                        _=lease.stopped()=>{ cancel.cancel(); future.await },
                        result=&mut future=>result,
                    }
                }
                Err(error) => Err(Error::Session(error)),
            };
            lease.close().await;
            let _ = tx.send(result);
        });
        let result = rx.await.map_err(|_| Error::Stopped)?;
        guard.disarm();
        result
    }
    async fn segment(
        &self,
        principal: Principal,
        id: Uuid,
        code: String,
        cancel: CancellationToken,
    ) -> Result<ResultValue, Error> {
        use crate::infrastructure::sandbox::Error as SandboxError;
        let mut count = 0;
        let capture = self.sandbox.capture();
        let future = async {
            let allowed = self
                .sources
                .allowed_sources(principal.workspace_id)
                .await
                .map_err(|_| SandboxError::Invalid("workspace scope unavailable".into()))?;
            let prelude = self.sources.prelude(allowed.as_deref()).await;
            let (mut run, mut step) = self
                .sandbox
                .start_captured_for(principal.token_id, &code, &prelude, capture.clone())
                .await?;
            loop {
                match step {
                    Step::Done { .. } => return Ok::<_, SandboxError>(step),
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
                        let results = stream::iter(calls.into_iter().enumerate())
                            .map(|(index, call)| {
                                self.sources.dispatch(
                                    id,
                                    first + index,
                                    call,
                                    self.limits.upstream_timeout,
                                    principal.workspace_id,
                                )
                            })
                            .buffer_unordered(self.limits.concurrent_calls)
                            .collect()
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
                Ok(Ok(step))=>("completed",None,Some(step)),
            }
        };
        let error = match error {
            Some(message) => Some(
                self.sources
                    .explain_error(message, principal.workspace_id)
                    .await,
            ),
            None => None,
        };
        let (result, output, output_dropped_bytes) = match done {
            Some(Step::Done {
                result,
                output,
                output_dropped_bytes,
            }) => (result, output, output_dropped_bytes),
            _ => {
                let (output, dropped) = Capture::snapshot(&capture);
                (RawValue::NULL.to_owned(), output, dropped)
            }
        };
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        let paths = sqlx::query_scalar!(
            "SELECT path FROM tool_calls WHERE execution_id=$1 ORDER BY seq",
            id
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(DbError::from)?;
        let title = title::title(&code, &paths);
        let stored_result = log::preview(result.get(), 16384);
        let stored_output = log::preview(&output, 16384);
        sqlx::query!("UPDATE tool_calls SET outcome='cancelled',error='Execution ended while call was in flight; upstream may have completed' WHERE execution_id=$1 AND outcome='running'",id).execute(&mut *tx).await.map_err(DbError::from)?;
        sqlx::query!("UPDATE executions SET status=$2,error=$3,tool_calls=$4,title=$5,result=$6,output=$7,finished_at=clock_timestamp() WHERE id=$1 AND status='running'",id,status,error,count as i32,title,stored_result,stored_output).execute(&mut *tx).await.map_err(DbError::from)?;
        sqlx::query!("UPDATE agent_sessions s SET last_seen_at=GREATEST(s.last_seen_at,clock_timestamp()),title=COALESCE(s.title,CASE WHEN NOT EXISTS(SELECT 1 FROM executions earlier JOIN executions current ON current.id=$1 WHERE earlier.session_id=s.id AND (earlier.started_at,earlier.id)<(current.started_at,current.id)) THEN $2 ELSE NULL END) WHERE s.id=(SELECT session_id FROM executions WHERE id=$1)",id,title).execute(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
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
