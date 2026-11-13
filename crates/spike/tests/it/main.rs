#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use codemode_spike::sandbox::{self, MontySandbox, Sandbox, Step};
use monty_pool::{PoolError, TurnEvent, on_print_sync};
use monty_types::MontyObject;
use serde_json::json;

#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn binary() -> PathBuf {
    std::env::var_os("CODEMODE_MONTY_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.tools/monty/bin/monty")
        })
}

async fn sandbox(deadline: Duration) -> MontySandbox {
    MontySandbox::new(&binary(), deadline, [73; 32])
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
    let step = sandbox::execute(&sb, code).await.unwrap();
    let Step::Done { result, output } = step else {
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
async fn gather_50_calls_execute_concurrently() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let start = Instant::now();
    let step = sandbox::execute(&sb, "r = await asyncio.gather(*[tools.stub.echo(value=i, delay_ms=100) for i in range(50)])\nreturn [x['value'] for x in r]").await.unwrap();
    let elapsed = start.elapsed();
    let Step::Done { result, .. } = step else {
        panic!("unfinished")
    };
    let result: Vec<u64> = serde_json::from_str(result.get()).unwrap();
    assert_eq!(result, (0..50).collect::<Vec<_>>());
    record(
        "concurrency",
        json!({"calls":50,"stub_delay_ms":100,"fanout":50,"elapsed_ms":elapsed.as_secs_f64()*1000.0}),
    );
    assert!(
        elapsed < Duration::from_millis(800),
        "50 calls took {elapsed:?}"
    );
    sb.pool.close().await;
}

#[tokio::test]
async fn serial_infinite_loop_hits_hard_deadline() {
    let sb = sandbox(Duration::from_millis(20)).await;
    let start = Instant::now();
    let err = sandbox::execute(&sb, "while True: pass").await.unwrap_err();
    assert!(
        matches!(err, sandbox::Error::Worker(PoolError::Timeout { .. })),
        "{err}"
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    let recovered = sandbox::execute(&sb, "return 42").await;
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
        let err = sandbox::execute(&sb, "while True: pass").await.unwrap_err();
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
        .start("r = await tools.stub.echo(value=42)\nreturn r['value']")
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
        .resume(healthy, sandbox::dispatch_stub(calls).await)
        .await
        .unwrap();
    let Step::Done { result, .. } = done else {
        panic!("unfinished neighbour")
    };
    assert_eq!(result.get(), "42");
    assert!(sandbox::execute(&sb, "return 42").await.is_ok());
    record(
        "memory",
        json!({"limit_bytes":sandbox::MEMORY,"requested_bytes":256*1024*1024,"error":error.to_string(),"worker_gone":true,"terminated_status":format!("{terminated:?}"),"neighbour_pid":healthy_pid,"neighbour_result":42,"elapsed_ms":start.elapsed().as_secs_f64()*1000.0}),
    );
    sb.pool.close().await;
}

#[tokio::test]
async fn deep_recursion_discards_only_its_worker() {
    let sb = sandbox(Duration::from_secs(1)).await;
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
    let Step::Done { result, .. } = sandbox::execute(&sb, "return 42").await.unwrap() else {
        panic!()
    };
    assert_eq!(result.get(), "42");
    record(
        "recursion",
        json!({"error":err.to_string(),"worker_gone":true,"pool_result_after":42}),
    );
    sb.pool.close().await;
}

#[tokio::test]
async fn killed_worker_is_reported_and_replaced() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let mut worker = sb.checkout().await.unwrap();
    let pid = worker.pid().unwrap();
    let start = Instant::now();
    let mut printer = on_print_sync(|_, _| {});
    let (result, _) = tokio::join!(
        worker.feed("while True: pass", vec![], vec![], false, &mut printer),
        async {
            // Qualification fault injection must let the real worker begin executing.
            tokio::time::sleep(Duration::from_millis(10)).await;
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
    let replacement = sb.checkout().await.unwrap();
    assert_ne!(replacement.pid(), Some(pid));
    replacement.finish().await.unwrap();
    assert!(sandbox::execute(&sb, "return 42").await.is_ok());
    record(
        "kill",
        json!({"error":error.to_string(),"replacement":true,"elapsed_ms":start.elapsed().as_secs_f64()*1000.0}),
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
        sandbox::execute(&sb, "return 42").await.unwrap();
    }
    let start = Instant::now();
    let mut samples = vec![rss_kib()];
    for i in 1..=10_000 {
        let Step::Done { result, .. } = sandbox::execute(&sb, "return 42").await.unwrap() else {
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
async fn serial_warm_run_starts_under_5ms() {
    let sb = sandbox(Duration::from_secs(1)).await;
    sandbox::execute(&sb, "return 42").await.unwrap();
    let mut micros = Vec::with_capacity(100);
    for _ in 0..100 {
        let start = Instant::now();
        let (run, step) = sb.start("return 42").await.unwrap();
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
async fn snapshot_restores_in_new_gateway_and_tampering_is_rejected() {
    let root = std::env::temp_dir().join(format!("codemode-snapshot-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let key = root.join("key");
    std::fs::write(&key, [73; 32]).unwrap();
    let snap = root.join("snapshot");
    for cmd in ["snapshot-write", "snapshot-read"] {
        let executable = std::env::var_os("CODEMODE_SPIKE_BIN")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_codemode-spike").into());
        let result = tokio::process::Command::new(executable)
            .arg(cmd)
            .arg(&snap)
            .env("CODEMODE_MONTY_BIN", binary())
            .env("CODEMODE_SNAPSHOT_KEY_FILE", &key)
            .output()
            .await
            .unwrap();
        assert!(
            result.status.success(),
            "{cmd}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let mut bytes = std::fs::read(&snap).unwrap();
    let size = bytes.len();
    *bytes.last_mut().unwrap() ^= 1;
    let sb = sandbox(Duration::from_secs(1)).await;
    assert!(matches!(
        sb.restore(&bytes).await,
        Err(sandbox::Error::Snapshot)
    ));
    record(
        "snapshot",
        json!({"new_gateway_process":true,"result":42,"snapshot_bytes":size,"tamper_rejected":true}),
    );
    sb.pool.close().await;
    std::fs::remove_dir_all(&root).unwrap();
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
    let Step::Done { result, .. } = sandbox::execute(&sb, "blocked = []\ntry:\n    open('/etc/passwd').read()\nexcept Exception:\n    blocked.append('file')\ntry:\n    import socket\nexcept Exception:\n    blocked.append('network')\nreturn blocked").await.unwrap() else { panic!() };
    assert_eq!(
        serde_json::from_str::<Vec<String>>(result.get()).unwrap(),
        ["file", "network"]
    );
    sb.pool.close().await;
}

mod protocol;
