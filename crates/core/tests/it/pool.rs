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
    MontySandbox::new(&binary(), deadline, 32768).await.unwrap()
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

#[tokio::test]
async fn call_cap_is_catchable_after_one_hundred_sequential_or_parallel_calls() {
    let sb = sandbox(Duration::from_secs(1)).await;
    for body in [
        "for i in range(101):\n        await tools.stub.echo(value=i)",
        "await asyncio.gather(*[tools.stub.echo(value=i) for i in range(101)])",
    ] {
        let code = format!(
            "try:\n    {body}\nexcept ToolError as e:\n    return str(e)\nreturn 'limit was not enforced'"
        );
        let Step::Done { result, .. } = execute(&sb, &code).await.unwrap() else {
            panic!("unfinished")
        };
        assert_eq!(result.get(), r#""tool-call limit (100) reached""#);
    }
    sb.pool.close().await;
}

#[tokio::test]
async fn pagination_and_projection_use_real_workers_and_preserve_call_budgets() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let prelude = format!(
        "{}\ntool = _Tool('stub.page', {{'from_': 'from'}}, 'full signature', {{'items':['data','items'], 'cursor':['data','nextCursor'], 'optional':False, 'extras':[]}})\n",
        include_str!("../../src/components/sources/prelude.py")
    );
    for (code, repeated, expected, expected_calls) in [
        (
            "return await tool.all(from_='date', _fields=['id'])",
            false,
            json!([{"id":1},{"id":2}]),
            2,
        ),
        (
            "try:\n    await tool.all(max_pages=1)\nexcept ToolError as e:\n    return str(e)",
            false,
            json!("pagination max_pages reached; narrow filters or increase max_pages"),
            1,
        ),
        (
            "try:\n    await tool.all()\nexcept ToolError as e:\n    return str(e)",
            true,
            json!("pagination cursor repeated; stopped to avoid a loop"),
            2,
        ),
        (
            "try:\n    await tool(_fields='id')\nexcept ToolError as e:\n    return str(e)",
            false,
            json!("_fields must be a list of field names"),
            0,
        ),
        (
            "return await tool(_fields=['id'])",
            false,
            json!({"data":{"items":[{"id":1}],"nextCursor":"next"}}),
            1,
        ),
        (
            "try:\n    await tool.all(max_pages=True)\nexcept ToolError as e:\n    return str(e)",
            false,
            json!("max_pages must be an integer from 1 to 100"),
            0,
        ),
    ] {
        let (mut run, mut step) = sb.start(code, &prelude).await.unwrap();
        let mut count = 0;
        loop {
            match step {
                Step::Done { result, .. } => {
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(result.get()).unwrap(),
                        expected,
                        "{code}"
                    );
                    assert_eq!(count, expected_calls, "{code}");
                    break;
                }
                Step::Calls(calls) => {
                    assert_eq!(calls.len(), 1);
                    count += 1;
                    let call = &calls[0];
                    let args: serde_json::Value = serde_json::from_str(call.args.get()).unwrap();
                    assert!(args.get("_fields").is_none());
                    assert!(args.get("max_pages").is_none());
                    if code.contains("from_") {
                        assert_eq!(args["from"], "date");
                    }
                    if count > 1 {
                        assert_eq!(args["cursor"], "next");
                    }
                    let response = json!({"data":{"items":[{"id":count,"title":"extra","labels":["keep only if requested"]}], "nextCursor":if count == 1 || repeated {json!("next")} else {json!(null)}}});
                    (run, step) = sb
                        .resume(
                            run,
                            vec![(
                                call.id,
                                Ok(serde_json::value::to_raw_value(&response).unwrap()),
                            )],
                        )
                        .await
                        .unwrap();
                }
            }
        }
    }
    let code = "await tool()\ntry:\n    await tool.all(max_pages=100)\nexcept ToolError as e:\n    return str(e)";
    let (mut run, mut step) = sb.start(code, &prelude).await.unwrap();
    let mut count = 0;
    loop {
        match step {
            Step::Done { result, .. } => {
                assert_eq!(
                    serde_json::from_str::<String>(result.get()).unwrap(),
                    "tool-call limit (100) reached"
                );
                assert_eq!(count, 100);
                break;
            }
            Step::Calls(calls) => {
                count += calls.len();
                assert!(count <= 100);
                let response = serde_json::value::to_raw_value(
                    &json!({"data":{"items":[],"nextCursor":count.to_string()}}),
                )
                .unwrap();
                (run, step) = sb
                    .resume(run, vec![(calls[0].id, Ok(response))])
                    .await
                    .unwrap();
            }
        }
    }
    sb.pool.close().await;
}

#[tokio::test]
async fn synchronous_broker_helpers_preserve_pending_async_calls() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let code = "async def first():\n    return await tools.stub.echo(value=1)\nasync def second():\n    r = json.loads(_call_sync('stub.echo', '{\"value\":2}'))\n    return await tools.stub.echo(value=r['value'])\nreturn await asyncio.gather(first(), second())";
    let Step::Done { result, .. } = execute(&sb, code).await.unwrap() else {
        panic!("unfinished")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(result.get()).unwrap(),
        json!([{"value":1},{"value":2}])
    );
    let Step::Done { result, .. } = execute(
        &sb,
        "try:\n    _call_sync('stub.fail', '{}')\nexcept ToolError as e:\n    return str(e)",
    )
    .await
    .unwrap() else {
        panic!("unfinished")
    };
    assert_eq!(
        serde_json::from_str::<String>(result.get()).unwrap(),
        "deliberate upstream failure"
    );
    sb.pool.close().await;
}

#[tokio::test]
async fn host_source_attributes_keep_python_callables_and_report_lazy_lookups() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let mut worker = sb.checkout().await.unwrap();
    let class = MontyObject::class_type(
        "_Source_fixture",
        monty_types::MontyUuid::from_bytes(*uuid::Uuid::new_v4().as_bytes()),
        true,
        false,
        [],
    );
    let proxy = MontyObject::class_instance(
        class,
        monty_types::MontyUuid::from_bytes(*uuid::Uuid::new_v4().as_bytes()),
        [],
    );
    let event = worker.feed("class Echo:\n    async def __call__(self):\n        return 42\nproxy.echo = Echo()\nasync def run():\n    result = await proxy.echo()\n    try:\n        proxy.no_such_tool\n    except RuntimeError as e:\n        return [result, str(e)]\nawait run()",vec![("proxy".into(),proxy)],vec![],false,&mut on_print_sync(|_,_|{})).await.unwrap();
    let TurnEvent::NameLookup {
        name, object_id, ..
    } = event
    else {
        panic!("expected lazy attribute lookup, got {event:?}")
    };
    assert_eq!(name, "no_such_tool");
    assert!(object_id.is_some());
    let done = worker
        .resume_name_lookup(
            monty_types::NameLookupResult::Error(monty_types::MontyException::new(
                monty_types::ExcType::RuntimeError,
                Some("did you mean: echo".into()),
            )),
            &mut on_print_sync(|_, _| {}),
        )
        .await
        .unwrap();
    let TurnEvent::Complete(result) = done else {
        panic!("expected completion")
    };
    assert_eq!(result.py_repr(), "[42, 'did you mean: echo']");
    worker.finish().await.unwrap();
    sb.pool.close().await;
}

#[tokio::test]
async fn pagination_accepts_integer_cursors_sort_companions_and_omitted_terminal_cursors() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let input = json!({"properties":{"cursor":{"type":"integer"},"cursor_date":{"type":"string"}}});
    let output = json!({"properties":{"items":{"type":"array","items":{}},"next_cursor":{"type":"integer"},"next_cursor_date":{"type":"string"}},"required":["items"]});
    let pagination =
        codemode_core::components::catalogue::stubs::pagination(&input, Some(&output)).unwrap();
    assert!(pagination.optional);
    assert_eq!(
        pagination.extras,
        vec![["cursor_date".to_string(), "next_cursor_date".to_string()]]
    );
    let metadata = codemode_core::components::catalogue::stubs::quote(
        &serde_json::to_string(&pagination).unwrap(),
    );
    let prelude = format!(
        "{}\ntool = _Tool('stub.page', {{}}, 'stub', json.loads({metadata}))\n",
        include_str!("../../src/components/sources/prelude.py")
    );
    let (run, step) = sb
        .start("return await tool.all(cursor=0, sort='date')", &prelude)
        .await
        .unwrap();
    let Step::Calls(first) = step else {
        panic!("expected page one")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(first[0].args.get()).unwrap()["cursor"],
        0
    );
    let page = json!({"items":[{"id":"first"}],"next_cursor":7,"next_cursor_date":"2026-01-02"});
    let (run, step) = sb
        .resume(
            run,
            vec![(
                first[0].id,
                Ok(serde_json::value::to_raw_value(&page).unwrap()),
            )],
        )
        .await
        .unwrap();
    let Step::Calls(second) = step else {
        panic!("expected page two")
    };
    let args: serde_json::Value = serde_json::from_str(second[0].args.get()).unwrap();
    assert_eq!(args["cursor"], 7);
    assert_eq!(args["cursor_date"], "2026-01-02");
    assert_eq!(args["sort"], "date");
    let (_, step) = sb
        .resume(
            run,
            vec![(
                second[0].id,
                Ok(serde_json::value::to_raw_value(&json!({"items":[{"id":"last"}]})).unwrap()),
            )],
        )
        .await
        .unwrap();
    let Step::Done { result, .. } = step else {
        panic!("expected completed pagination")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(result.get()).unwrap(),
        json!([{"id":"first"},{"id":"last"}])
    );
    sb.pool.close().await;
}

#[tokio::test]
async fn moneybag_recipe_uses_exact_decimal_postings_and_integer_pagination() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let prelude = format!(
        "{}\nclass Ledger:\n    def __init__(self):\n        self.entries_list = _Tool('ledger.entries_list', {{'from_':'from'}}, 'stub', {{'items':['items'],'cursor':['next_cursor'],'optional':True,'extras':[]}})\nclass Tools:\n    def __init__(self):\n        self.ledger = Ledger()\ntools = Tools()\nargs = {{'from':'2026-01-01','to':'2026-01-31','account':'Expenses:Food'}}\n",
        include_str!("../../src/components/sources/prelude.py")
    );
    let code = include_str!("../../src/components/scripts/recipes/moneybag_monthly_totals.py")
        .replace("$SOURCE", "ledger");
    let (run, step) = sb.start(&code, &prelude).await.unwrap();
    let Step::Calls(first) = step else {
        panic!("expected first page")
    };
    let args: serde_json::Value = serde_json::from_str(first[0].args.get()).unwrap();
    assert_eq!(args["from"], "2026-01-01");
    assert_eq!(args["account"], "Expenses:Food");
    let page = json!({"items":[{"date":"2026-01-02","postings":[{"account":"Expenses:Food","amount":{"v":"0.1","c":"EUR"}},{"account":"Assets:Bank","amount":{"v":"-0.1","c":"EUR"}}]}],"next_cursor":7});
    let (run, step) = sb
        .resume(
            run,
            vec![(
                first[0].id,
                Ok(serde_json::value::to_raw_value(&page).unwrap()),
            )],
        )
        .await
        .unwrap();
    let Step::Calls(second) = step else {
        panic!("expected second page")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(second[0].args.get()).unwrap()["cursor"],
        7
    );
    let page = json!({"items":[{"date":"2026-01-03","postings":[{"account":"Expenses:Food","amount":{"v":"0.2","c":"EUR"}},{"account":"Expenses:Food","amount":{"v":"-0.01","c":"EUR"}},{"account":"Expenses:Food:Restaurant","amount":{"v":"5.00","c":"EUR"}},{"account":"Expenses:FoodExtra","amount":{"v":"999","c":"EUR"}},{"account":"Expenses:Food","amount":{"v":"1.005","c":"USD"}}]}]});
    let (_, step) = sb
        .resume(
            run,
            vec![(
                second[0].id,
                Ok(serde_json::value::to_raw_value(&page).unwrap()),
            )],
        )
        .await
        .unwrap();
    let Step::Done { result, .. } = step else {
        panic!("expected totals")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(result.get()).unwrap(),
        json!({"account":"Expenses:Food","months":{"2026-01":{"EUR":"5.29","USD":"1.005"}}})
    );
    sb.pool.close().await;
}

#[tokio::test]
async fn nudge_recipe_resolves_names_before_one_create_and_validates_args_first() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let prelude = format!(
        "{}\nclass Issues:\n    def __init__(self):\n        self.list_teams = _Tool('issues.list_teams', {{}}, '', {{'items':['data','items'],'cursor':['data','nextCursor'],'optional':True,'extras':[]}})\n        self.list_issue_statuses = _Tool('issues.list_issue_statuses', {{}}, '', {{'items':['data','items'],'cursor':['data','nextCursor'],'optional':True,'extras':[]}})\n        self.create_issue = _Tool('issues.create_issue', {{}}, '', None)\nclass Tools:\n    def __init__(self):\n        self.issues = Issues()\ntools = Tools()\nargs = {{'teamKey':'EVL','statusName':'Open','title':'Recipe issue'}}\n",
        include_str!("../../src/components/sources/prelude.py")
    );
    let code = include_str!("../../src/components/scripts/recipes/nudge_create_issue.py")
        .replace("$SOURCE", "issues");
    let (mut run, mut step) = sb.start(&code, &prelude).await.unwrap();
    for (path, response) in [
        (
            "issues.list_teams",
            json!({"data":{"items":[{"id":"team","key":"EVL"}]}}),
        ),
        (
            "issues.list_issue_statuses",
            json!({"data":{"items":[{"id":"open","name":"Open","category":"unstarted"}]}}),
        ),
        (
            "issues.create_issue",
            json!({"data":{"id":"created","identifier":"EVL-1","title":"Recipe issue","statusId":"open"}}),
        ),
    ] {
        let Step::Calls(calls) = step else {
            panic!("expected {path}")
        };
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].path, path);
        let args: serde_json::Value = serde_json::from_str(calls[0].args.get()).unwrap();
        if path == "issues.list_issue_statuses" {
            assert_eq!(args["teamId"], "team");
        }
        if path == "issues.create_issue" {
            assert_eq!(
                args,
                json!({"teamId":"team","statusId":"open","title":"Recipe issue"})
            );
        }
        (run, step) = sb
            .resume(
                run,
                vec![(
                    calls[0].id,
                    Ok(serde_json::value::to_raw_value(&response).unwrap()),
                )],
            )
            .await
            .unwrap();
    }
    let Step::Done { result, .. } = step else {
        panic!("expected created issue")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(result.get()).unwrap()["id"],
        "created"
    );
    let invalid = format!("{prelude}\nargs['priority'] = 99\n");
    let error = sb
        .start(&code, &invalid)
        .await
        .err()
        .expect("invalid args must fail before any broker call");
    assert!(error.to_string().contains("priority must be"));
    sb.pool.close().await;
}

#[tokio::test]
async fn host_source_call_operator_can_return_native_search_results() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let mut worker = sb.checkout().await.unwrap();
    let id = monty_types::MontyUuid::from_bytes(*uuid::Uuid::new_v4().as_bytes());
    let class = MontyObject::class_type(
        "_Source_search",
        monty_types::MontyUuid::from_bytes(*uuid::Uuid::new_v4().as_bytes()),
        true,
        false,
        [(
            MontyObject::string("__call__"),
            MontyObject::function("_inline_search", None),
        )],
    );
    let proxy = MontyObject::class_instance(class, id, []);
    let event = worker.feed("async def echo(value):\n    return value\nproxy.echo = echo\nassert await proxy.echo('known') == 'known'\nawait proxy('query')",vec![("proxy".into(),proxy)],vec![],false,&mut on_print_sync(|_,_|{})).await.unwrap();
    let TurnEvent::FunctionCall {
        function_name,
        object_id,
        args,
        call_id,
        ..
    } = event
    else {
        panic!("expected source call, got {event:?}")
    };
    assert_eq!(function_name, "__call__");
    assert_eq!(object_id, Some(id));
    assert_eq!(args.args().len(), 1);
    assert_eq!(args.arg(0).unwrap().as_str(), Some("query"));
    let event = worker
        .resume(
            monty_pool::ResumeValue::Future,
            &mut on_print_sync(|_, _| {}),
        )
        .await
        .unwrap();
    assert!(matches!(event, TurnEvent::ResolveFutures { .. }));
    let done = worker
        .resume_futures(
            vec![(
                call_id,
                monty_pool::ResumeValue::Return(MontyObject::list([MontyObject::string("found")])),
            )],
            &mut on_print_sync(|_, _| {}),
        )
        .await
        .unwrap();
    let TurnEvent::Complete(result) = done else {
        panic!("expected search hits")
    };
    assert_eq!(result.py_repr(), "['found']");
    worker.finish().await.unwrap();
    sb.pool.close().await;
}

#[tokio::test]
async fn source_proxy_setup_does_not_reduce_the_one_hundred_call_budget() {
    let sb = sandbox(Duration::from_secs(1)).await;
    let mut prelude = PRELUDE.to_string();
    for i in 0..64 {
        prelude.push_str(&format!("source_{i} = _source_proxy('source_{i}')\n"));
    }
    prelude.push_str("root = _source_proxy('')\n");
    let (mut run, mut step) = sb
        .start(
            "for i in range(100):\n    await tools.stub.echo(value=i)\nreturn 100",
            &prelude,
        )
        .await
        .unwrap();
    let mut count = 0;
    loop {
        match step {
            Step::Done { result, .. } => {
                assert_eq!(result.get(), "100");
                assert_eq!(count, 100);
                break;
            }
            Step::Calls(calls) => {
                count += calls.len();
                (run, step) = sb.resume(run, dispatch_stub(calls).await).await.unwrap();
            }
        }
    }
    sb.pool.close().await;
}
