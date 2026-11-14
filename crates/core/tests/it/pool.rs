use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use codemode_core::infrastructure::sandbox::{
    self, CallResult, MontySandbox, PendingCall, Sandbox, Step,
};
use monty_pool::{PoolError, TurnEvent, on_print_sync};
use monty_types::MontyObject;
use serde_json::json;

fn binary() -> PathBuf {
    std::env::var_os("CODEMODE_MONTY_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.tools/monty/bin/monty")
        })
}

async fn sandbox(deadline: Duration) -> MontySandbox {
    MontySandbox::new(&binary(), deadline, [73; 32], 32768)
        .await
        .unwrap()
}

fn record(name: &str, value: serde_json::Value) {
    if let Some(dir) = std::env::var_os("CODEMODE_RESULTS_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            PathBuf::from(dir).join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
    println!("{name}: {value}");
}

#[tokio::test]
async fn python_calling_shape_and_catchable_errors() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let code = "r = await tools.stub.echo(value=21)\ntry:\n    await tools.stub.fail()\nexcept ToolError as e:\n    print(str(e))\nreturn r['value'] * 2";
    let step = execute(&sb, code).await.unwrap();
    let Step::Done { result, output, .. } = step else {
        panic!("unfinished")
    };
    assert_eq!(result.get(), "42");
    assert_eq!(output, "deliberate upstream failure\n");
    record(
        "python",
        json!({"result":42,"await":true,"tools.source.tool":true,"ToolError_alias":"RuntimeError","return":"wrapped async function"}),
    );
    sb.pool.close().await;
}

#[tokio::test]
async fn gather_yields_all_fifty_calls_in_one_batch() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let (run, step) = sb.start_fixture("r = await asyncio.gather(*[tools.stub.echo(value=i) for i in range(50)])\nreturn [x['value'] for x in r]").await.unwrap();
    let Step::Calls(calls) = step else {
        panic!("expected batch")
    };
    assert_eq!(calls.len(), 50);
    let (_, step) = sb.resume(run, dispatch_stub(calls).await).await.unwrap();
    let Step::Done { result, .. } = step else {
        panic!("expected result")
    };
    assert_eq!(
        serde_json::from_str::<Vec<u64>>(result.get()).unwrap(),
        (0..50).collect::<Vec<_>>()
    );
    sb.pool.close().await;
}

#[tokio::test]
async fn serial_infinite_loop_hits_hard_deadline() {
    let sb = sandbox(Duration::from_millis(20)).await;
    let start = Instant::now();
    let err = execute(&sb, "while True: pass").await.unwrap_err();
    assert!(
        matches!(err, sandbox::Error::Worker(PoolError::Timeout { .. })),
        "{err}"
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    let recovered = execute(&sb, "return 42").await;
    assert!(recovered.is_ok(), "pool recovery: {recovered:?}");
    sb.pool.close().await;
}

#[tokio::test]
#[ignore = "slow: 100 independent deadline and worker-replacement trials"]
async fn serial_deadline_stops_infinite_loop_100_times() {
    let sb = sandbox(Duration::from_millis(20)).await;
    let mut worst = Duration::ZERO;
    for _ in 0..100 {
        let start = Instant::now();
        let err = execute(&sb, "while True: pass").await.unwrap_err();
        assert!(
            matches!(err, sandbox::Error::Worker(PoolError::Timeout { .. })),
            "{err}"
        );
        worst = worst.max(start.elapsed());
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    record(
        "deadline",
        json!({"trials":100,"stopped":100,"deadline_ms":20,"worst_total_ms":worst.as_secs_f64()*1000.0}),
    );
    sb.pool.close().await;
}

#[tokio::test]
async fn memory_limit_discards_only_its_worker() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let (healthy, step) = sb
        .start_fixture("r = await tools.stub.echo(value=42)\nreturn r['value']")
        .await
        .unwrap();
    let Step::Calls(calls) = step else {
        panic!("expected suspended neighbour")
    };
    let healthy_pid = healthy.worker.as_ref().unwrap().pid().unwrap();
    let mut abusive = sb.checkout().await.unwrap();
    let abusive_pid = abusive.pid().unwrap();
    let start = Instant::now();
    let error = abusive
        .feed(
            "a = 'x' * (256 * 1024 * 1024)",
            vec![],
            vec![],
            false,
            &mut on_print_sync(|_, _| {}),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("MemoryError"), "{error}");
    // A soft interpreter limit leaves an unsafe heap: terminate and confirm OS exit.
    let terminated = abusive.terminate().await;
    assert!(!PathBuf::from(format!("/proc/{abusive_pid}")).exists());
    let (_, done) = sb
        .resume(healthy, dispatch_stub(calls).await)
        .await
        .unwrap();
    let Step::Done { result, .. } = done else {
        panic!("unfinished neighbour")
    };
    assert_eq!(result.get(), "42");
    assert!(execute(&sb, "return 42").await.is_ok());
    record(
        "memory",
        json!({"limit_bytes":sandbox::MEMORY,"requested_bytes":256*1024*1024,"error":error.to_string(),"worker_gone":true,"terminated_status":format!("{terminated:?}"),"neighbour_pid":healthy_pid,"neighbour_result":42,"elapsed_ms":start.elapsed().as_secs_f64()*1000.0}),
    );
    sb.pool.close().await;
}

#[tokio::test]
async fn deep_recursion_discards_only_its_worker() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let (healthy, step) = sb
        .start_fixture("r = await tools.stub.echo(value=42)\nreturn r['value']")
        .await
        .unwrap();
    let Step::Calls(calls) = step else {
        panic!("expected suspended neighbour")
    };
    let healthy_pid = healthy.worker.as_ref().unwrap().pid().unwrap();
    let mut abusive = sb.checkout().await.unwrap();
    let pid = abusive.pid().unwrap();
    let err = abusive
        .feed(
            "def recurse():\n    return recurse()\nrecurse()",
            vec![],
            vec![],
            false,
            &mut on_print_sync(|_, _| {}),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("RecursionError"), "{err}");
    abusive.terminate().await;
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    let (_, done) = sb
        .resume(healthy, dispatch_stub(calls).await)
        .await
        .unwrap();
    let Step::Done { result, .. } = done else {
        panic!("unfinished neighbour")
    };
    assert_eq!(result.get(), "42");
    let Step::Done { result, .. } = execute(&sb, "return 42").await.unwrap() else {
        panic!()
    };
    assert_eq!(result.get(), "42");
    record(
        "recursion",
        json!({"error":err.to_string(),"worker_gone":true,"neighbour_pid":healthy_pid,"neighbour_result":42,"pool_result_after":42}),
    );
    sb.pool.close().await;
}

#[tokio::test]
async fn killed_worker_is_reported_and_replaced() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let (healthy, step) = sb
        .start_fixture("r = await tools.stub.echo(value=42)\nreturn r['value']")
        .await
        .unwrap();
    let Step::Calls(calls) = step else {
        panic!("expected suspended neighbour")
    };
    let healthy_pid = healthy.worker.as_ref().unwrap().pid().unwrap();
    let mut worker = sb.checkout().await.unwrap();
    let pid = worker.pid().unwrap();
    let start = Instant::now();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let mut started = Some(started_tx);
    let mut printer = on_print_sync(|_, _| {
        if let Some(sender) = started.take() {
            let _ = sender.send(());
        }
    });
    let (result, _) = tokio::join!(
        worker.feed(
            "print('running')\nwhile True: pass",
            vec![],
            vec![],
            false,
            &mut printer
        ),
        async {
            // Kill only after a print frame proves the real worker is executing.
            tokio::time::timeout(Duration::from_secs(1), started_rx)
                .await
                .unwrap()
                .unwrap();
            assert!(
                tokio::process::Command::new("kill")
                    .args(["-KILL", &pid.to_string()])
                    .status()
                    .await
                    .unwrap()
                    .success()
            );
        }
    );
    let error = result.unwrap_err();
    assert!(matches!(error, PoolError::Crashed { .. }), "{error}");
    let replacement_pids = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let pids = sb.pool.idle_worker_pids();
            if !pids.is_empty() {
                break pids;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the pool must refill without another checkout");
    assert!(
        replacement_pids
            .iter()
            .all(|id| *id != pid && *id != healthy_pid)
    );
    let replacement = sb.checkout().await.unwrap();
    let replacement_pid = replacement.pid().unwrap();
    assert_ne!(replacement_pid, pid);
    assert_ne!(replacement_pid, healthy_pid);
    replacement.finish().await.unwrap();
    let (_, done) = sb
        .resume(healthy, dispatch_stub(calls).await)
        .await
        .unwrap();
    let Step::Done { result, .. } = done else {
        panic!("unfinished neighbour")
    };
    assert_eq!(result.get(), "42");
    assert!(execute(&sb, "return 42").await.is_ok());
    record(
        "kill",
        json!({"error":error.to_string(),"killed_pid":pid,"automatic_replacement_pid":replacement_pid,"neighbour_pid":healthy_pid,"neighbour_result":42,"replacement":true,"elapsed_ms":start.elapsed().as_secs_f64()*1000.0}),
    );
    sb.pool.close().await;
}

fn rss_kib() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    status
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")
                .map(|s| s.split_whitespace().next().unwrap().parse().unwrap())
        })
        .unwrap()
}

#[tokio::test]
#[ignore = "slow: 10,000 real worker checkouts for RSS qualification"]
async fn sequential_10000_runs_have_flat_gateway_memory() {
    let sb = sandbox(Duration::from_secs(1)).await;
    for _ in 0..200 {
        execute(&sb, "return 42").await.unwrap();
    }
    let start = Instant::now();
    let mut samples = vec![rss_kib()];
    for i in 1..=10_000 {
        let Step::Done { result, .. } = execute(&sb, "return 42").await.unwrap() else {
            panic!()
        };
        assert_eq!(result.get(), "42");
        if i % 1000 == 0 {
            samples.push(rss_kib());
        }
    }
    let growth = *samples.last().unwrap() as i64 - samples[0] as i64;
    record(
        "soak",
        json!({"runs":10000,"warmup":200,"rss_kib":samples,"growth_kib":growth,"elapsed_s":start.elapsed().as_secs_f64()}),
    );
    assert!(growth < 2048, "gateway RSS grew {growth} KiB");
    sb.pool.close().await;
}

#[tokio::test]
#[ignore = "slow: 100 real-worker latency samples on deployment hardware without compiler contention"]
async fn serial_warm_run_starts_under_5ms() {
    let sb = sandbox(Duration::from_secs(1)).await;
    execute(&sb, "return 42").await.unwrap();
    let mut micros = Vec::with_capacity(100);
    for _ in 0..100 {
        let start = Instant::now();
        let (run, step) = sb.start_fixture("return 42").await.unwrap();
        let elapsed = start.elapsed();
        assert!(matches!(step, Step::Done { .. }));
        drop(run);
        micros.push(elapsed.as_micros());
    }
    micros.sort();
    record(
        "latency",
        json!({"samples":100,"median_us":micros[50],"p95_us":micros[95],"max_us":micros[99],"architecture":std::env::consts::ARCH}),
    );
    assert!(micros[99] < 5000, "warm start took {} us", micros[99]);
    sb.pool.close().await;
}

#[tokio::test]
async fn snapshot_restores_in_a_fresh_pool_and_rejects_tampering() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let (run, step) = sb
        .start_fixture(
            "print('before')\nr = await tools.stub.echo(value=21)\nreturn r['value'] * 2",
        )
        .await
        .unwrap();
    let Step::Calls(calls) = step else {
        panic!("expected call")
    };
    let snapshot = sb.snapshot(run).await.unwrap();
    sb.pool.close().await;
    let restored = sandbox(Duration::from_secs(1)).await;
    let mut tampered = snapshot.clone();
    *tampered.last_mut().unwrap() ^= 1;
    assert!(matches!(
        restored.restore(&tampered).await,
        Err(sandbox::Error::Snapshot)
    ));
    let (run, _) = restored.restore(&snapshot).await.unwrap();
    let (_, step) = restored
        .resume(run, dispatch_stub(calls).await)
        .await
        .unwrap();
    let Step::Done { result, output, .. } = step else {
        panic!("expected result")
    };
    assert_eq!(result.get(), "42");
    assert_eq!(output, "before\n");
    restored.pool.close().await;
}

#[tokio::test]
async fn worker_environment_is_empty_and_files_network_are_denied() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let mut worker = sb.checkout().await.unwrap();
    let pid = worker.pid().unwrap();
    assert!(
        std::fs::read(format!("/proc/{pid}/environ"))
            .unwrap()
            .is_empty()
    );
    let event = worker
        .feed(
            "await f()",
            vec![("f".into(), MontyObject::function("f", None))],
            vec![],
            false,
            &mut on_print_sync(|_, _| {}),
        )
        .await
        .unwrap();
    assert!(matches!(event, TurnEvent::FunctionCall { .. }));
    drop(worker);
    let Step::Done { result, .. } = execute(&sb, "blocked = []\ntry:\n    open('/etc/passwd').read()\nexcept Exception:\n    blocked.append('file')\ntry:\n    import socket\nexcept Exception:\n    blocked.append('network')\nreturn blocked").await.unwrap() else { panic!() };
    assert_eq!(
        serde_json::from_str::<Vec<String>>(result.get()).unwrap(),
        ["file", "network"]
    );
    sb.pool.close().await;
}

const PRELUDE: &str = "import asyncio\nimport json\nToolError = RuntimeError\nclass _Stub:\n    async def echo(self, **kwargs):\n        return json.loads(await _call('stub.echo', json.dumps(kwargs)))\n    async def fail(self):\n        return json.loads(await _call('stub.fail', '{}'))\nclass _Tools:\n    def __init__(self):\n        self.stub = _Stub()\ntools = _Tools()\n";

trait FixtureStart {
    async fn start_fixture(&self, code: &str) -> Result<(sandbox::Run, Step), sandbox::Error>;
}
impl FixtureStart for MontySandbox {
    async fn start_fixture(&self, code: &str) -> Result<(sandbox::Run, Step), sandbox::Error> {
        self.start(code, PRELUDE).await
    }
}

async fn dispatch_stub(calls: Vec<PendingCall>) -> Vec<(u32, CallResult)> {
    calls
        .into_iter()
        .map(|call| {
            let result = match call.path.as_str() {
                "stub.echo" => Ok(call.args),
                "stub.fail" => Err("deliberate upstream failure".into()),
                _ => Err("unknown tool".into()),
            };
            (call.id, result)
        })
        .collect()
}

async fn execute(sb: &MontySandbox, code: &str) -> Result<Step, sandbox::Error> {
    let (mut run, mut step) = sb.start_fixture(code).await?;
    loop {
        match step {
            Step::Done { .. } => return Ok(step),
            Step::Calls(calls) => {
                (run, step) = sb.resume(run, dispatch_stub(calls).await).await?;
            }
        }
    }
}

#[tokio::test]
async fn a_worker_process_never_crosses_authenticated_caller_boundaries() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();
    let code = "return await tools.stub.echo(value=42)";
    let (run, step) = sb.start_for(a, code, PRELUDE).await.unwrap();
    let Step::Calls(calls) = step else {
        panic!("expected pending call")
    };
    let a_pid = run.worker.as_ref().unwrap().pid().unwrap();
    let (_, done) = sb.resume(run, dispatch_stub(calls).await).await.unwrap();
    assert!(matches!(done, Step::Done { .. }));
    let (run, step) = sb.start_for(a, code, PRELUDE).await.unwrap();
    let Step::Calls(calls) = step else {
        panic!("expected pending call")
    };
    assert_eq!(
        run.worker.as_ref().unwrap().pid(),
        Some(a_pid),
        "same caller can reuse its warm worker"
    );
    let (_, done) = sb.resume(run, dispatch_stub(calls).await).await.unwrap();
    assert!(matches!(done, Step::Done { .. }));
    let (run, step) = sb.start_for(b, code, PRELUDE).await.unwrap();
    let Step::Calls(calls) = step else {
        panic!("expected pending call")
    };
    assert_ne!(
        run.worker.as_ref().unwrap().pid(),
        Some(a_pid),
        "another caller must get a different process"
    );
    let (_, done) = sb.resume(run, dispatch_stub(calls).await).await.unwrap();
    assert!(matches!(done, Step::Done { .. }));
    sb.pool.close().await;
}
