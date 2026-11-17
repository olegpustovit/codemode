use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

use monty_pool::{
    Checkout, Pool, PoolConfig, PoolError, ReplConfig, ResumeValue, TurnEvent, on_print_sync,
};
use monty_types::{
    ExcType, MontyException, MontyObject, MontyUuid, NameLookupResult, ResourceLimits,
};
use serde_json::value::RawValue;

pub const CALL_CAP: usize = 100;
const SOURCE_PROXIES: usize = 65; // The supported 64 sources plus the root namespace.
pub const MEMORY: usize = 64 * 1024 * 1024;
pub const OUTPUT: usize = 32 * 1024;
const MAX_FRAME: usize = 72 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Worker(#[source] PoolError),
    #[error("invalid worker message: {0}")]
    Invalid(String),
    #[error("source exceeds 64 KB")]
    Source,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PendingCall {
    pub id: u32,
    pub path: String,
    pub args: Box<RawValue>,
}

#[derive(Debug)]
pub enum Step {
    Done {
        result: Box<RawValue>,
        output: String,
        output_dropped_bytes: usize,
    },
    Calls(Vec<PendingCall>),
}

#[derive(Debug, thiserror::Error)]
pub enum CallFailure {
    #[error("{0}")]
    Tool(String),
    #[error("{0}")]
    Permission(String),
}
impl From<String> for CallFailure {
    fn from(message: String) -> Self {
        Self::Tool(message)
    }
}
impl From<&str> for CallFailure {
    fn from(message: &str) -> Self {
        Self::Tool(message.into())
    }
}
pub type CallResult = Result<Box<RawValue>, CallFailure>;

pub trait Sandbox {
    type Run: Send;
    fn start(
        &self,
        code: &str,
        prelude: &str,
    ) -> impl Future<Output = Result<(Self::Run, Step), Error>> + Send;
    fn resume(
        &self,
        run: Self::Run,
        results: Vec<(u32, CallResult)>,
    ) -> impl Future<Output = Result<(Self::Run, Step), Error>> + Send;
    fn start_for(
        &self,
        _caller: Uuid,
        code: &str,
        prelude: &str,
    ) -> impl Future<Output = Result<(Self::Run, Step), Error>> + Send {
        self.start(code, prelude)
    }
    fn capture(&self) -> Arc<Mutex<Capture>> {
        Arc::new(Mutex::new(Capture::default()))
    }
    fn start_captured_for(
        &self,
        caller: Uuid,
        code: &str,
        prelude: &str,
        _output: Arc<Mutex<Capture>>,
    ) -> impl Future<Output = Result<(Self::Run, Step), Error>> + Send {
        self.start_for(caller, code, prelude)
    }
}

pub struct MontySandbox {
    pub pool: Pool,
    repl: ReplConfig,
    output_budget: usize,
    turn_deadline: Duration,
    owners: Mutex<Owners>,
}

#[derive(Default)]
struct Owners {
    latest: Option<u64>,
    callers: BTreeMap<u64, Uuid>,
}
impl Owners {
    fn accept(&mut self, worker: u64, caller: Uuid) -> bool {
        if let Some(owner) = self.callers.get(&worker) {
            return *owner == caller;
        }
        // Evicted or out-of-order IDs are discarded, never mistaken for clean processes.
        if self.latest.is_some_and(|latest| worker <= latest) {
            return false;
        }
        self.latest = Some(worker);
        self.callers.insert(worker, caller);
        if self.callers.len() > 64 {
            self.callers.pop_first();
        }
        true
    }
}

pub struct Run {
    pub worker: Option<Checkout>,
    output: Arc<Mutex<Capture>>,
    calls: usize,
    pending: Vec<PendingCall>,
    reply: ReplyMode,
    proxies: BTreeMap<MontyUuid, String>,
    native_json: BTreeSet<u32>,
}
#[derive(Clone, Copy)]
enum ReplyMode {
    Futures,
    Sync(u32),
    Lookup(u32),
}

impl MontySandbox {
    pub async fn new(
        binary: &Path,
        deadline: Duration,
        output_budget: usize,
    ) -> Result<Self, Error> {
        let mut config = PoolConfig::subprocess(binary);
        config.min_processes = 2;
        config.max_processes = 8;
        // Configure includes cold process isolation; execution turns keep their own hard deadline.
        config.request_timeout = Some(deadline.max(Duration::from_secs(1)));
        config.checkout_timeout = Some(Duration::from_secs(1));
        // Valid results can approach the worker's memory ceiling.
        config.max_decode_bytes = MAX_FRAME;
        let pool = Pool::new(config).await?;
        let repl = ReplConfig {
            script_name: "execution.py".into(),
            limits: Some(
                ResourceLimits::default()
                    .max_memory(MEMORY)
                    .max_suspensions(2 * CALL_CAP + SOURCE_PROXIES + 16),
            ),
            ..Default::default()
        };
        Ok(Self {
            pool,
            repl,
            output_budget,
            turn_deadline: deadline,
            owners: Mutex::new(Owners::default()),
        })
    }

    // Erase SDK command futures at this boundary to keep incremental code generation bounded.
    fn turn<'a, T: Send + 'a>(
        &'a self,
        operation: impl Future<Output = Result<T, PoolError>> + Send + 'a,
    ) -> futures_util::future::BoxFuture<'a, Result<T, Error>> {
        Box::pin(async move {
            tokio::time::timeout(self.turn_deadline, operation)
                .await
                .map_err(|_| {
                    Error::Worker(PoolError::Timeout {
                        timeout: self.turn_deadline,
                    })
                })?
                .map_err(Error::Worker)
        })
    }

    pub async fn checkout(&self) -> Result<Checkout, Error> {
        self.checkout_for(Uuid::nil()).await
    }
    async fn checkout_for(&self, caller: Uuid) -> Result<Checkout, Error> {
        loop {
            let worker = self.pool.checkout(&self.repl).await?;
            let id = worker
                .worker_id()
                .ok_or_else(|| Error::Invalid("worker has no process identity".into()))?;
            if self
                .owners
                .lock()
                .map_err(|_| Error::Invalid("worker ownership state unavailable".into()))?
                .accept(id, caller)
            {
                return Ok(worker);
            }
            drop(worker);
        }
    }

    async fn advance(&self, mut run: Run, mut event: TurnEvent) -> Result<(Run, Step), Error> {
        loop {
            let mut printer = on_print_sync(|_, text| Capture::append_shared(&run.output, text));
            let worker = run
                .worker
                .as_mut()
                .ok_or_else(|| Error::Invalid("worker already finished".into()))?;
            event = match event {
                TurnEvent::Complete(value) => {
                    let result = value
                        .as_ref()
                        .as_str()
                        .ok_or_else(|| Error::Invalid("result must be JSON text".into()))?;
                    let result = RawValue::from_string(result.into())
                        .map_err(|e| Error::Invalid(e.to_string()))?;
                    drop(printer);
                    if let Some(worker) = run.worker.take() {
                        self.turn(worker.finish()).await?;
                    }
                    let output = Capture::take(&run.output);
                    return Ok((
                        run,
                        Step::Done {
                            result,
                            output: output.0,
                            output_dropped_bytes: output.1,
                        },
                    ));
                }
                TurnEvent::FunctionCall {
                    function_name,
                    args,
                    call_id,
                    object_id,
                    ..
                } => {
                    if run.pending.iter().any(|call| call.id == call_id) {
                        return Err(Error::Invalid("duplicate callback identity".into()));
                    }
                    if let Some(source) = object_id.and_then(|id| run.proxies.get(&id)) {
                        if function_name != "__call__" {
                            if run.calls >= CALL_CAP {
                                event = self
                                    .turn(worker.resume(
                                        ResumeValue::Error(MontyException::new(
                                            ExcType::RuntimeError,
                                            Some(format!("tool-call limit ({CALL_CAP}) reached")),
                                        )),
                                        &mut printer,
                                    ))
                                    .await?;
                                continue;
                            }
                            let path = if source.is_empty() {
                                function_name
                            } else {
                                format!("{source}.{function_name}")
                            };
                            run.calls += 1;
                            let args =
                                serde_json::value::to_raw_value(&serde_json::json!({"path":path}))
                                    .map_err(|_| {
                                        Error::Invalid("could not encode unknown tool".into())
                                    })?;
                            run.reply = ReplyMode::Sync(call_id);
                            drop(printer);
                            return Ok((
                                run,
                                Step::Calls(vec![PendingCall {
                                    id: call_id,
                                    path: "@codemode/unknown".into(),
                                    args,
                                }]),
                            ));
                        }
                        if source != "search" {
                            event = self.turn(worker.resume(ResumeValue::Error(MontyException::new(ExcType::TypeError,Some("source namespaces are not callable; call a tool or tools.search(query)".into()))),&mut printer)).await?;
                            continue;
                        }
                    }
                    let native_search = function_name == "__call__"
                        && object_id
                            .and_then(|id| run.proxies.get(&id))
                            .is_some_and(|source| source == "search");
                    let (path, args) = if native_search {
                        let query = match (args.args().len(), args.kwargs().len()) {
                            (1, 0) => args.arg(0),
                            (0, 1) => args.kwarg("query"),
                            _ => None,
                        }
                        .and_then(|value| value.as_str().map(str::to_owned));
                        let Some(query) = query else {
                            event = self
                                .turn(worker.resume(
                                    ResumeValue::Error(MontyException::new(
                                        ExcType::TypeError,
                                        Some("tools.search requires query: str".into()),
                                    )),
                                    &mut printer,
                                ))
                                .await?;
                            continue;
                        };
                        (
                            "@codemode/search".into(),
                            serde_json::value::to_raw_value(&serde_json::json!({"query":query}))
                                .map_err(|_| {
                                    Error::Invalid("could not encode search query".into())
                                })?,
                        )
                    } else {
                        if !matches!(function_name.as_str(), "_call" | "_call_sync")
                            || object_id.is_some()
                            || args.args().len() != 2
                            || args.kwargs().len() != 0
                        {
                            return Err(Error::Invalid("invalid callback".into()));
                        }
                        let path = args
                            .arg(0)
                            .and_then(|value| value.as_str().map(str::to_owned))
                            .ok_or_else(|| Error::Invalid("tool path must be text".into()))?;
                        let text = args
                            .arg(1)
                            .and_then(|value| value.as_str().map(str::to_owned))
                            .ok_or_else(|| {
                                Error::Invalid("tool arguments must be JSON text".into())
                            })?;
                        let args = RawValue::from_string(text)
                            .map_err(|error| Error::Invalid(error.to_string()))?;
                        if !args.get().starts_with('{') {
                            return Err(Error::Invalid("arguments must be an object".into()));
                        }
                        (path, args)
                    };
                    if run.calls >= CALL_CAP {
                        event = self
                            .turn(worker.resume(
                                ResumeValue::Error(MontyException::new(
                                    ExcType::RuntimeError,
                                    Some(format!("tool-call limit ({CALL_CAP}) reached")),
                                )),
                                &mut printer,
                            ))
                            .await?;
                        continue;
                    }
                    run.calls += 1;
                    let call = PendingCall {
                        id: call_id,
                        path,
                        args,
                    };
                    if function_name == "_call_sync" {
                        run.reply = ReplyMode::Sync(call_id);
                        drop(printer);
                        return Ok((run, Step::Calls(vec![call])));
                    }
                    if native_search {
                        run.native_json.insert(call_id);
                    }
                    run.pending.push(call);
                    self.turn(worker.resume(ResumeValue::Future, &mut printer))
                        .await?
                }
                TurnEvent::ResolveFutures {
                    pending_call_ids, ..
                } => {
                    if !pending_matches(&run.pending, &pending_call_ids) {
                        return Err(Error::Invalid("unmatched pending calls".into()));
                    }
                    drop(printer);
                    let pending = std::mem::take(&mut run.pending);
                    return Ok((run, Step::Calls(pending)));
                }
                TurnEvent::OsCall { .. } => {
                    self.turn(worker.resume(ResumeValue::NotHandled, &mut printer))
                        .await?
                }
                TurnEvent::NameLookup {
                    name, object_id, ..
                } => {
                    if let Some(source) = object_id.and_then(|id| run.proxies.get(&id)) {
                        if run.calls >= CALL_CAP {
                            self.turn(worker.resume_name_lookup(
                                NameLookupResult::Error(MontyException::new(
                                    ExcType::RuntimeError,
                                    Some(format!("tool-call limit ({CALL_CAP}) reached")),
                                )),
                                &mut printer,
                            ))
                            .await?
                        } else {
                            let path = if source.is_empty() {
                                name
                            } else {
                                format!("{source}.{name}")
                            };
                            run.calls += 1;
                            let id = u32::MAX - run.calls as u32;
                            if run.pending.iter().any(|call| call.id == id) {
                                return Err(Error::Invalid(
                                    "lookup identity collides with pending callback".into(),
                                ));
                            }
                            let args =
                                serde_json::value::to_raw_value(&serde_json::json!({"path":path}))
                                    .map_err(|_| {
                                        Error::Invalid("could not encode unknown tool".into())
                                    })?;
                            run.reply = ReplyMode::Lookup(id);
                            drop(printer);
                            return Ok((
                                run,
                                Step::Calls(vec![PendingCall {
                                    id,
                                    path: "@codemode/unknown".into(),
                                    args,
                                }]),
                            ));
                        }
                    } else {
                        self.turn(worker.resume_name_lookup(None, &mut printer))
                            .await?
                    }
                }
            };
        }
    }
}

// Batches are capped at 100 calls; validate IDs without allocating a second collection.
fn pending_matches(pending: &[PendingCall], ids: &[u32]) -> bool {
    pending.len() == ids.len()
        && pending.iter().enumerate().all(|(i, call)| {
            ids.contains(&call.id) && pending[..i].iter().all(|other| other.id != call.id)
        })
}

impl Sandbox for MontySandbox {
    type Run = Run;

    async fn start(&self, code: &str, prelude: &str) -> Result<(Run, Step), Error> {
        self.start_for(Uuid::nil(), code, prelude).await
    }
    fn capture(&self) -> Arc<Mutex<Capture>> {
        Arc::new(Mutex::new(Capture {
            budget: self.output_budget,
            ..Default::default()
        }))
    }
    async fn start_for(
        &self,
        caller: Uuid,
        code: &str,
        prelude: &str,
    ) -> Result<(Run, Step), Error> {
        self.start_captured_for(caller, code, prelude, self.capture())
            .await
    }
    async fn start_captured_for(
        &self,
        caller: Uuid,
        code: &str,
        prelude: &str,
        output: Arc<Mutex<Capture>>,
    ) -> Result<(Run, Step), Error> {
        if code.len() > 64 * 1024 {
            return Err(Error::Source);
        }
        let mut worker = self.checkout_for(caller).await?;
        let mut prelude_event = self
            .turn(worker.feed(
                prelude.to_owned(),
                vec![
                    ("_call".into(), MontyObject::function("_call", None)),
                    (
                        "_source_proxy".into(),
                        MontyObject::function("_source_proxy", None),
                    ),
                    (
                        "_call_sync".into(),
                        MontyObject::function("_call_sync", None),
                    ),
                ],
                vec![],
                false,
                &mut on_print_sync(|_, _| {}),
            ))
            .await?;
        let mut proxies = BTreeMap::new();
        while let TurnEvent::FunctionCall {
            function_name,
            args,
            object_id,
            ..
        } = prelude_event
        {
            if function_name != "_source_proxy"
                || object_id.is_some()
                || args.args().len() != 1
                || args.kwargs().len() != 0
                || proxies.len() >= SOURCE_PROXIES
            {
                return Err(Error::Invalid("invalid prelude callback".into()));
            }
            let alias = args
                .arg(0)
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| Error::Invalid("source alias must be text".into()))?;
            if alias.len() > 128
                || !alias
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                || proxies.values().any(|value| value == &alias)
            {
                return Err(Error::Invalid("invalid source alias".into()));
            }
            let class_id = MontyUuid::from_bytes(*Uuid::new_v4().as_bytes());
            let id = MontyUuid::from_bytes(*Uuid::new_v4().as_bytes());
            let attrs = if alias == "search" {
                vec![(
                    MontyObject::string("__call__"),
                    MontyObject::function("_inline_search", None),
                )]
            } else {
                Vec::new()
            };
            let class = MontyObject::class_type(
                if alias.is_empty() {
                    "_Tools".into()
                } else {
                    format!("_Source_{alias}")
                },
                class_id,
                true,
                false,
                attrs,
            );
            let proxy = MontyObject::class_instance(class, id, []);
            proxies.insert(id, alias);
            prelude_event = self
                .turn(worker.resume(ResumeValue::Return(proxy), &mut on_print_sync(|_, _| {})))
                .await?;
        }
        if !matches!(prelude_event, TurnEvent::Complete(_)) {
            return Err(Error::Invalid("unexpected prelude callback".into()));
        }
        let mut source = String::new();
        source.push_str("async def __codemode_main():\n");
        if code.trim().is_empty() {
            source.push_str("    pass\n");
        }
        for line in code.lines() {
            source.push_str("    ");
            source.push_str(line);
            source.push('\n');
        }
        source.push_str("json.dumps(await __codemode_main())\n");
        let event = self
            .turn(worker.feed(
                source,
                vec![
                    ("_call".into(), MontyObject::function("_call", None)),
                    (
                        "_call_sync".into(),
                        MontyObject::function("_call_sync", None),
                    ),
                ],
                vec![],
                false,
                &mut on_print_sync(|_, text| Capture::append_shared(&output, text)),
            ))
            .await?;
        self.advance(
            Run {
                worker: Some(worker),
                output,
                calls: 0,
                pending: Vec::new(),
                reply: ReplyMode::Futures,
                proxies,
                native_json: BTreeSet::new(),
            },
            event,
        )
        .await
    }

    async fn resume(
        &self,
        mut run: Run,
        results: Vec<(u32, CallResult)>,
    ) -> Result<(Run, Step), Error> {
        let mut answers: Vec<_> = results
            .into_iter()
            .map(|(id, result)| {
                let native = run.native_json.remove(&id);
                Ok((
                    id,
                    match result {
                        Ok(value) => ResumeValue::Return(if native {
                            native_json(&value)?
                        } else {
                            MontyObject::string(value.get())
                        }),
                        Err(error) => ResumeValue::Error(MontyException::new(
                            match error {
                                CallFailure::Permission(_) => ExcType::PermissionError,
                                CallFailure::Tool(_) => ExcType::RuntimeError,
                            },
                            Some(error.to_string()),
                        )),
                    },
                ))
            })
            .collect::<Result<_, Error>>()?;
        let worker = run
            .worker
            .as_mut()
            .ok_or_else(|| Error::Invalid("worker already finished".into()))?;
        let mut printer = on_print_sync(|_, text| Capture::append_shared(&run.output, text));
        let mode = std::mem::replace(&mut run.reply, ReplyMode::Futures);
        let event = match mode {
            ReplyMode::Futures => {
                self.turn(worker.resume_futures(answers, &mut printer))
                    .await?
            }
            ReplyMode::Sync(id) | ReplyMode::Lookup(id) => {
                let Some((reply_id, answer)) = answers.pop() else {
                    return Err(Error::Invalid("missing synchronous reply".into()));
                };
                if reply_id != id || !answers.is_empty() {
                    return Err(Error::Invalid("unmatched synchronous reply".into()));
                }
                if matches!(mode, ReplyMode::Lookup(_)) {
                    let ResumeValue::Error(error) = answer else {
                        return Err(Error::Invalid(
                            "unknown-tool lookup must return an error".into(),
                        ));
                    };
                    self.turn(
                        worker.resume_name_lookup(NameLookupResult::Error(error), &mut printer),
                    )
                    .await?
                } else {
                    self.turn(worker.resume(answer, &mut printer)).await?
                }
            }
        };
        drop(printer);
        self.advance(run, event).await
    }
}

fn native_json(raw: &RawValue) -> Result<MontyObject, Error> {
    fn convert(value: serde_json::Value) -> Result<MontyObject, Error> {
        use serde_json::Value;
        Ok(match value {
            Value::Null => MontyObject::none(),
            Value::Bool(value) => MontyObject::bool(value),
            Value::String(value) => MontyObject::string(value),
            Value::Number(value) => {
                if let Some(value) = value.as_i64() {
                    MontyObject::int(value)
                } else if let Some(value) = value.as_u64() {
                    MontyObject::bigint(value.into())
                } else {
                    MontyObject::float(
                        value
                            .as_f64()
                            .ok_or_else(|| Error::Invalid("invalid search number".into()))?,
                    )
                }
            }
            Value::Array(values) => MontyObject::list(
                values
                    .into_iter()
                    .map(convert)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Value::Object(values) => MontyObject::dict(
                values
                    .into_iter()
                    .map(|(key, value)| Ok((MontyObject::string(key), convert(value)?)))
                    .collect::<Result<Vec<_>, Error>>()?,
            ),
        })
    }
    convert(
        serde_json::from_str(raw.get())
            .map_err(|_| Error::Invalid("invalid native search reply".into()))?,
    )
}

pub struct Capture {
    budget: usize,
    text: String,
    dropped: usize,
}
impl Default for Capture {
    fn default() -> Self {
        Self {
            budget: OUTPUT,
            text: String::new(),
            dropped: 0,
        }
    }
}
impl Capture {
    fn append_shared(output: &Arc<Mutex<Self>>, text: &str) {
        output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .append(text);
    }
    pub(crate) fn snapshot(output: &Arc<Mutex<Self>>) -> (String, usize) {
        let output = output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (output.text.clone(), output.dropped)
    }
    fn take(output: &Arc<Mutex<Self>>) -> (String, usize) {
        let mut output = output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            std::mem::take(&mut output.text),
            std::mem::take(&mut output.dropped),
        )
    }

    fn append(&mut self, text: &str) {
        let mut n = text.len().min(self.budget.saturating_sub(self.text.len()));
        while !text.is_char_boundary(n) {
            n -= 1;
        }
        self.text.push_str(&text[..n]);
        self.dropped = self.dropped.saturating_add(text.len() - n);
    }
}

impl From<PoolError> for Error {
    fn from(error: PoolError) -> Self {
        Self::Worker(error)
    }
}

impl Error {
    pub fn agent_message(&self) -> String {
        match self {
            Self::Worker(PoolError::Runtime(error)) => {
                let line = error
                    .traceback()
                    .iter()
                    .rev()
                    .find(|f| f.frame_name.as_deref() == Some("__codemode_main"))
                    .map(|f| f.start.line.saturating_sub(1));
                format!(
                    "{}{}",
                    error.summary(),
                    line.map(|n| format!(" (execution.py line {n})"))
                        .unwrap_or_default()
                )
            }
            Self::Worker(PoolError::Timeout { .. }) => {
                "execution deadline exceeded; worker terminated".into()
            }
            Self::Worker(PoolError::Crashed { .. }) => {
                "sandbox worker exited unexpectedly; other executions are unaffected".into()
            }
            Self::Worker(_) => "sandbox worker failed".into(),
            _ => self.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn caller_ownership_cannot_be_reassigned_or_revived_after_cache_eviction() {
        let mut owners = super::Owners::default();
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        assert!(owners.accept(0, a));
        assert!(!owners.accept(0, b));
        assert!(owners.accept(0, a));
        for id in 1..1000 {
            assert!(owners.accept(id, a));
        }
        assert_eq!(owners.callers.len(), 64);
        assert!(!owners.accept(0, a));
        assert!(!owners.accept(0, b));
        assert!(!owners.accept(999, b));
        assert!(owners.accept(1000, b));
    }
    use super::*;
    #[test]
    fn worker_wire_codec_reuses_buffers_and_enforces_decode_budget() {
        use monty_proto::{decode_frame_with_budget, encode_framed_into, pb};
        let frame = pb::Feed {
            code: "x".repeat(65536),
            ..Default::default()
        };
        let mut buffer = Vec::with_capacity(65552);
        encode_framed_into(&frame, &mut buffer).unwrap();
        let encoding = allocation_counter::measure(|| {
            for _ in 0..100 {
                encode_framed_into(&frame, &mut buffer).unwrap();
            }
        });
        // The pool uses this exact codec: zero encode allocations after warm-up.
        assert_eq!(encoding.count_total, 0);
        let decoding = allocation_counter::measure(|| {
            let decoded: pb::Feed = decode_frame_with_budget(&buffer[4..], 131072).unwrap();
            assert_eq!(decoded.code, frame.code);
        });
        // One owned string payload, independent of its length; no intermediate tree.
        assert_eq!(decoding.count_total, 1);
        assert!(decode_frame_with_budget::<pb::Feed>(&buffer[4..], 8).is_err());
    }
    #[test]
    fn pending_ids_are_a_bijection_without_allocations() {
        let calls = |ids: &[u32]| {
            ids.iter()
                .map(|&id| PendingCall {
                    id,
                    path: "example.echo".into(),
                    args: RawValue::from_string("{}".into()).unwrap(),
                })
                .collect::<Vec<_>>()
        };
        let valid = calls(&[1, 2]);
        let allocations = allocation_counter::measure(|| {
            assert!(pending_matches(&valid, &[2, 1]));
            assert!(!pending_matches(&valid, &[1, 1]));
            assert!(!pending_matches(&valid, &[1]));
            assert!(!pending_matches(&valid, &[1, 3]));
        });
        assert_eq!(allocations.count_total, 0);
        assert!(!pending_matches(&calls(&[1, 1]), &[1, 2]));
    }
    #[test]
    fn captures_utf8_and_counts_every_dropped_byte() {
        let mut capture = Capture::default();
        capture.append(&"x".repeat(OUTPUT - 1));
        capture.append("é");
        capture.append("abc");
        assert_eq!(capture.text.len(), OUTPUT);
        assert_eq!(capture.dropped, 4);
    }
}
