use std::{path::Path, time::Duration};

use hmac::{Hmac, Mac};
use monty_pool::{
    Checkout, Pool, PoolConfig, PoolError, ReplConfig, ResumeValue, TurnEvent, on_print_sync,
};
use monty_types::{ExcType, MontyException, MontyObject, ResourceLimits};
use serde_json::value::RawValue;
use sha2::Sha256;

pub const MEMORY: usize = 64 * 1024 * 1024;
pub const OUTPUT: usize = 32 * 1024;
const MAX_SNAPSHOT: usize = 72 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Worker(#[from] PoolError),
    #[error("invalid worker message: {0}")]
    Invalid(String),
    #[error("snapshot authentication failed")]
    Snapshot,
    #[error("source exceeds 64 KB")]
    Source,
}

#[derive(Debug)]
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
    },
    Calls(Vec<PendingCall>),
}

pub type CallResult = Result<Box<RawValue>, String>;

pub trait Sandbox {
    type Run;
    fn start(&self, code: &str) -> impl Future<Output = Result<(Self::Run, Step), Error>> + Send;
    fn resume(
        &self,
        run: Self::Run,
        results: Vec<(u32, CallResult)>,
    ) -> impl Future<Output = Result<(Self::Run, Step), Error>> + Send;
    fn snapshot(&self, run: Self::Run) -> impl Future<Output = Result<Vec<u8>, Error>> + Send;
    fn restore(
        &self,
        snapshot: &[u8],
    ) -> impl Future<Output = Result<(Self::Run, Step), Error>> + Send;
}

pub struct MontySandbox {
    pub pool: Pool,
    repl: ReplConfig,
    key: [u8; 32],
    prelude: String,
}

pub struct Run {
    pub worker: Option<Checkout>,
    output: String,
    calls: usize,
}

// The Python wrappers keep dynamic JSON parsing in the worker; the gateway carries RawValue.
const PRELUDE: &str = r#"import asyncio
import json
ToolError = RuntimeError
class _Stub:
    async def echo(self, **kwargs):
        return json.loads(await _call("stub.echo", json.dumps(kwargs)))
    async def fail(self, **kwargs):
        return json.loads(await _call("stub.fail", json.dumps(kwargs)))
class _Tools:
    def __init__(self):
        self.stub = _Stub()
tools = _Tools()
"#;

impl MontySandbox {
    pub async fn new(binary: &Path, deadline: Duration, key: [u8; 32]) -> Result<Self, Error> {
        let mut config = PoolConfig::subprocess(binary);
        config.min_processes = 2;
        config.max_processes = 8;
        config.request_timeout = Some(deadline);
        config.checkout_timeout = Some(Duration::from_secs(1));
        config.max_decode_bytes = 4 * 1024 * 1024;
        let pool = Pool::new(config).await?;
        let repl = ReplConfig {
            script_name: "execution.py".into(),
            limits: Some(
                ResourceLimits::default()
                    .max_memory(MEMORY)
                    .max_suspensions(110),
            ),
            ..Default::default()
        };
        Ok(Self {
            pool,
            repl,
            key,
            prelude: PRELUDE.into(),
        })
    }

    pub fn set_trial_tools(&mut self, tools: &[(String, Vec<String>)]) -> Result<(), Error> {
        let mut prelude = String::from("import asyncio\nimport json\nToolError = RuntimeError\n");
        for (source, names) in tools {
            if !identifier(source) || names.iter().any(|name| !identifier(name)) {
                return Err(Error::Invalid(
                    "trial source and tool names must be Python identifiers".into(),
                ));
            }
            prelude.push_str(&format!("class _Source_{source}:\n"));
            if names.is_empty() {
                prelude.push_str("    pass\n");
            }
            for name in names {
                let path = serde_json::to_string(&format!("{source}.{name}"))
                    .map_err(|e| Error::Invalid(e.to_string()))?;
                prelude.push_str(&format!("    async def {name}(self, **kwargs):\n        return json.loads(await _call({path}, json.dumps(kwargs)))\n"));
            }
        }
        prelude.push_str("class _Tools:\n    def __init__(self):\n");
        if tools.is_empty() {
            prelude.push_str("        pass\n");
        }
        for (source, _) in tools {
            prelude.push_str(&format!("        self.{source} = _Source_{source}()\n"));
        }
        prelude.push_str("tools = _Tools()\n");
        self.prelude = prelude;
        Ok(())
    }

    pub async fn checkout(&self) -> Result<Checkout, Error> {
        Ok(self.pool.checkout(&self.repl).await?)
    }

    async fn advance(&self, mut run: Run, mut event: TurnEvent) -> Result<(Run, Step), Error> {
        let mut pending = Vec::new();
        loop {
            let mut printer = on_print_sync(|_, text| append_output(&mut run.output, text));
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
                        worker.finish().await?;
                    }
                    let output = std::mem::take(&mut run.output);
                    return Ok((run, Step::Done { result, output }));
                }
                TurnEvent::FunctionCall {
                    function_name,
                    args,
                    call_id,
                    object_id,
                    ..
                } => {
                    run.calls += 1;
                    if run.calls > 100
                        || function_name != "_call"
                        || object_id.is_some()
                        || args.args().len() != 2
                        || args.kwargs().len() != 0
                    {
                        return Err(Error::Invalid(
                            "invalid callback or call limit exceeded".into(),
                        ));
                    }
                    let path = args
                        .arg(0)
                        .and_then(|a| a.as_str().map(str::to_owned))
                        .ok_or_else(|| Error::Invalid("tool path must be text".into()))?;
                    let text = args
                        .arg(1)
                        .and_then(|a| a.as_str().map(str::to_owned))
                        .ok_or_else(|| Error::Invalid("tool arguments must be JSON text".into()))?;
                    let args =
                        RawValue::from_string(text).map_err(|e| Error::Invalid(e.to_string()))?;
                    if !args.get().starts_with('{') {
                        return Err(Error::Invalid("arguments must be an object".into()));
                    }
                    pending.push(PendingCall {
                        id: call_id,
                        path,
                        args,
                    });
                    worker.resume(ResumeValue::Future, &mut printer).await?
                }
                TurnEvent::ResolveFutures {
                    pending_call_ids, ..
                } => {
                    if pending.len() != pending_call_ids.len()
                        || pending.iter().any(|c| !pending_call_ids.contains(&c.id))
                    {
                        return Err(Error::Invalid("unmatched pending calls".into()));
                    }
                    drop(printer);
                    return Ok((run, Step::Calls(pending)));
                }
                TurnEvent::OsCall { .. } => {
                    worker.resume(ResumeValue::NotHandled, &mut printer).await?
                }
                TurnEvent::NameLookup { .. } => {
                    worker.resume_name_lookup(None, &mut printer).await?
                }
            };
        }
    }
}

impl Sandbox for MontySandbox {
    type Run = Run;

    async fn start(&self, code: &str) -> Result<(Run, Step), Error> {
        if code.len() > 64 * 1024 {
            return Err(Error::Source);
        }
        let mut worker = self.checkout().await?;
        let mut source = self.prelude.clone();
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
        let mut output = String::new();
        let event = worker
            .feed(
                source,
                vec![("_call".into(), MontyObject::function("_call", None))],
                vec![],
                false,
                &mut on_print_sync(|_, text| append_output(&mut output, text)),
            )
            .await?;
        self.advance(
            Run {
                worker: Some(worker),
                output,
                calls: 0,
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
        let answers = results
            .into_iter()
            .map(|(id, result)| {
                (
                    id,
                    match result {
                        Ok(value) => ResumeValue::Return(MontyObject::string(value.get())),
                        Err(message) => ResumeValue::Error(MontyException::new(
                            ExcType::RuntimeError,
                            Some(message),
                        )),
                    },
                )
            })
            .collect();
        let worker = run
            .worker
            .as_mut()
            .ok_or_else(|| Error::Invalid("worker already finished".into()))?;
        let event = worker
            .resume_futures(
                answers,
                &mut on_print_sync(|_, text| append_output(&mut run.output, text)),
            )
            .await?;
        self.advance(run, event).await
    }

    async fn snapshot(&self, mut run: Run) -> Result<Vec<u8>, Error> {
        let worker = run
            .worker
            .as_mut()
            .ok_or_else(|| Error::Invalid("worker already finished".into()))?;
        let dump = worker.dump().await?;
        let mut bytes = Vec::with_capacity(44 + run.output.len() + dump.len());
        bytes.extend_from_slice(&[0; 32]);
        bytes.extend_from_slice(&(run.calls as u32).to_le_bytes());
        bytes.extend_from_slice(&(run.output.len() as u32).to_le_bytes());
        bytes.extend_from_slice(run.output.as_bytes());
        bytes.extend_from_slice(&dump);
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).map_err(|_| Error::Snapshot)?;
        mac.update(&bytes[32..]);
        bytes[..32].copy_from_slice(&mac.finalize().into_bytes());
        if bytes.len() > MAX_SNAPSHOT {
            return Err(Error::Snapshot);
        }
        Ok(bytes)
    }

    async fn restore(&self, snapshot: &[u8]) -> Result<(Run, Step), Error> {
        if snapshot.len() < 40 || snapshot.len() > MAX_SNAPSHOT {
            return Err(Error::Snapshot);
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).map_err(|_| Error::Snapshot)?;
        mac.update(&snapshot[32..]);
        mac.verify_slice(&snapshot[..32])
            .map_err(|_| Error::Snapshot)?;
        let calls =
            u32::from_le_bytes(snapshot[32..36].try_into().map_err(|_| Error::Snapshot)?) as usize;
        let output_len =
            u32::from_le_bytes(snapshot[36..40].try_into().map_err(|_| Error::Snapshot)?) as usize;
        if output_len > OUTPUT || calls > 100 || 40 + output_len >= snapshot.len() {
            return Err(Error::Snapshot);
        }
        let output = std::str::from_utf8(&snapshot[40..40 + output_len])
            .map_err(|_| Error::Snapshot)?
            .to_owned();
        let mut worker = self.checkout().await?;
        let (event, _) = worker
            .restore(
                snapshot[40 + output_len..].to_vec(),
                vec![],
                &mut on_print_sync(|_, _| {}),
            )
            .await?;
        let event = event.ok_or_else(|| Error::Invalid("expected suspended snapshot".into()))?;
        // Restored futures refer to the batch saved by the host, not newly emitted calls.
        let step = match event {
            TurnEvent::ResolveFutures { .. } => Step::Calls(vec![]),
            other => {
                return Err(Error::Invalid(format!(
                    "unexpected restored event: {other:?}"
                )));
            }
        };
        Ok((
            Run {
                worker: Some(worker),
                output,
                calls,
            },
            step,
        ))
    }
}

fn append_output(out: &mut String, text: &str) {
    let mut n = text.len().min(OUTPUT.saturating_sub(out.len()));
    while !text.is_char_boundary(n) {
        n -= 1;
    }
    out.push_str(&text[..n]);
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
        && ![
            "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
            "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
            "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
            "return", "try", "while", "with", "yield",
        ]
        .contains(&value)
}

pub async fn dispatch_stub(calls: Vec<PendingCall>) -> Vec<(u32, CallResult)> {
    use futures_util::{StreamExt, stream};
    stream::iter(calls)
        .map(|call| async move {
            let result = match call.path.as_str() {
                "stub.echo" => {
                    #[derive(serde::Deserialize)]
                    struct Args {
                        #[serde(default)]
                        delay_ms: u64,
                    }
                    match serde_json::from_str::<Args>(call.args.get()) {
                        Ok(args) if args.delay_ms <= 1000 => {
                            tokio::time::sleep(Duration::from_millis(args.delay_ms)).await;
                            Ok(call.args)
                        }
                        _ => Err("invalid delay_ms (expected integer 0..1000)".into()),
                    }
                }
                "stub.fail" => Err("deliberate upstream failure".into()),
                _ => Err("unknown tool".into()),
            };
            (call.id, result)
        })
        .buffer_unordered(50)
        .collect()
        .await
}

pub async fn execute(sandbox: &MontySandbox, code: &str) -> Result<Step, Error> {
    let (mut run, mut step) = sandbox.start(code).await?;
    loop {
        match step {
            Step::Done { .. } => return Ok(step),
            Step::Calls(calls) => {
                (run, step) = sandbox.resume(run, dispatch_stub(calls).await).await?
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::identifier;

    #[test]
    fn trial_names_cannot_inject_python_or_use_keywords() {
        assert!(identifier("list_issues"));
        assert!(identifier("source2"));
        for name in ["", "2source", "return", "a.b", "x():\n    pass", "x-y"] {
            assert!(!identifier(name));
        }
    }
}
