# Phase 0: Monty and MCP qualification

2026-11-13. Implementation plan [revision 5](https://artifacts.microapps.space/a/saqdh2xm7jcd).

Status: all nine criteria pass on `oracle-vm`, and the real-server Python trial passes after fixture corrections. The owner's reading of this report is the remaining Phase 0 gate. This is the Phase 0 spike, not the completed gateway.

## Measured criteria

Primary measurements below are on `oracle-vm` (aarch64, Ubuntu glibc 2.35, kernel 6.8.0-1049-oracle). Development-machine measurements are retained separately. The runnable cases are in `crates/spike/tests/it/`; all run through `cargo t`. Exact samples and earlier failures are retained under `docs/measurements/`.

| # | Criterion | Measured evidence | Result |
|---|---|---|---|
| 1 | 50 gathered 100 ms calls overlap | 141.9 ms on Oracle with fan-out 50, results in original order | Pass on Oracle |
| 2 | Stop an infinite loop in 100 independent runs | 100/100 `PoolError::Timeout` at a configured 20 ms per-turn deadline; worst checkout + execution + cleanup total 26.4 ms | Pass on Oracle |
| 3 | Memory excess ends only its worker | Request for a 256 MiB string rejected at the 64 MiB interpreter limit; supervisor then terminates and reaps that worker; an already-paused neighbour returns 42 | Pass on Oracle |
| 4 | Deep recursion affects only its worker | `RecursionError` at the default depth, offending process terminated and reaped; a neighbouring paused execution and a subsequent run both return 42 | Pass on Oracle |
| 5 | Worker killed during execution yields a clean error and replacement | SIGKILL becomes `PoolError::Crashed`; automatic refill creates a new PID before any new checkout; a neighbouring execution and a subsequent run both succeed | Pass on Oracle |
| 6 | Flat gateway memory over 10,000 runs | After 200 warmups: RSS 7,096 → 7,228 KiB, +132 KiB, flat after the second sample; 10,000 runs in 2.31 s; full series in `soak.json` | Pass on Oracle |
| 7 | Warm start below 5 ms on arm64 | 100 starts: median 263 μs, p95 1,497 μs, maximum 2,233 μs | Pass on Oracle |
| 8 | Snapshot survives a gateway restart; tampering rejected | Separate write/read gateway processes, fresh pool and worker; result 42, preserved output, signed dump about 7.5 KiB. Flipping a dump byte yields `Snapshot` before checkout/restore | Pass on Oracle |
| 9 | One stateless endpoint serves both protocol eras | Official rmcp clients: `server/discover` in 2026-07-28 and initialize in 2025-11-25; each lists and calls the echo tool | Pass on Oracle |

The memory test deliberately distinguishes a soft interpreter limit from a process abort. A soft `MemoryError` or `RecursionError` does not itself kill Monty. The adapter drops failed checkouts; the isolation qualification explicitly calls `terminate()` and confirms OS exit. Finishing/reusing a checkout after a resource-limit failure would be unsafe.

The pool's `max_memory` is allocated-byte accounting, not an RSS cap. Its worker allocator adds 4 MB of headroom (32 MB with type checking); stack, allocator metadata and fragmentation add RSS beyond that count. Phase 4 needs a measured cgroup cap before other people use this service.

Earlier runs under concurrent compiler/test workloads recorded an 8,036 μs warm-start maximum and a 20 ms post-timeout recovery failure. They remain in `docs/measurements/local/`. Latency/deadline qualifications now run exclusively via the runner's `serial_` convention, with the same thresholds and no retries. These results establish unloaded latency, not a shared-host p99 guarantee.

## Versions and real API

- Rust 1.96.0, exact toolchain pin.
- `monty-pool`, `monty-types`, `monty-proto`, `monty-fs`, `monty-macros`: exactly 1.5.0, including transitive pins. Version 1.6.0 requires Rust 1.98, so this spike uses the matching 1.5.0 runtime/toolchain family.
- Worker: `pydantic-monty-runtime==1.5.0`, installed separately from PyPI; `monty --version` checked. `monty-pool` expects its binary path and does not supply the executable.
- `rmcp` and `rmcp-macros`: exactly 3.10.0.
- The gateway dependency tree contains the pool/types/protocol crates, not the in-process Monty interpreter.

The verified pool calls are:

```rust
let mut cfg = PoolConfig::subprocess(absolute_worker_path);
cfg.request_timeout = Some(deadline); // no timeout by default
cfg.checkout_timeout = Some(Duration::from_secs(1));
cfg.max_decode_bytes = 4 * 1024 * 1024;
let pool = Pool::new(cfg).await?;
let mut worker = pool.checkout(&repl_config).await?;
let event = worker.feed(source, inputs, vec![], false, &mut on_print).await?;
// FunctionCall -> resume(Future), accumulating calls until ResolveFutures.
// Run the batch in Tokio; resume_futures(Vec<(call_id, Return(...) | Error(...))>).
let bytes = worker.dump().await?;
// On a NEW checkout, after checking host HMAC:
let (event, script_name) = worker.restore(bytes, vec![], &mut on_print).await?;
// Success only: finish() resets and returns a worker to the warm pool.
// Failure/cancellation: drop kills it; terminate().await confirms exit.
```

`TurnEvent` variants are `Complete`, `FunctionCall`, `ResolveFutures`, `OsCall`, `NameLookup`. The adapter validates its callback name, receiver, argument shape, call count, JSON arguments and matching pending IDs. The pool's decoder validates every worker frame with a decode budget; malformed frames discard their worker. No mounts, host OS handlers or import handlers are granted. An integration case checks the actual worker environment is empty and file/network operations are denied.

The subprocess implementation calls `env_clear()`. Credentials exist only in the host trial MCP clients and are referenced by owner-only files. They are never passed as worker inputs.

Snapshot envelope: HMAC-SHA256 over the call count, captured output and opaque Monty dump; verify before taking a worker. The host must retain the batch of pending calls alongside the dump. The spike retains a known call ID in its fixture; Phase 3 must persist the actual batch, caller, hostname, decisions, expiry and single-use claim in Postgres. Restart restore continues from the pending call and does not replay source code.

## Python calling convention

The requested syntax works:

```python
items = await tools.nudge.list_issues(limit=20)
try:
    details = await asyncio.gather(*[
        tools.nudge.get_issue(id=i["id"])
        for i in items["data"]["items"] or []
    ])
except ToolError as e:
    return {"error": str(e)}
return [d["data"]["title"] for d in details]
```

- `tools` and source wrappers are ordinary sandbox Python classes. Every method delegates to the sole host callback with a path and JSON keyword arguments.
- Host JSON is carried as `RawValue` by the adapter, decoded inside the worker. The official MCP client's structured-content API itself materializes JSON in the host; eliminating that tree in the production broker remains a Phase 1 hot-path requirement.
- Code is wrapped in an `async def` and awaited, enabling the requested top-level `return`. Traceback lines currently include the generated prelude/wrapper offset; Phase 1 must report lines from the user's source.
- Monty cannot define a custom exception subclass. `ToolError` is an alias of `RuntimeError`, and host failures resume with `ExcType::RuntimeError`. Consequently `except ToolError` also catches other runtime errors.
- No inheritance, generator functions, third-party imports or general event loop; `asyncio.gather` is supported. Model-facing instructions must explain these restrictions.
- The adapter bounds source at 64 KiB, callbacks at 100, output at 32 KiB, and pool size at 8. Full execution-wide deadlines, permits, audit, result truncation contracts and cancellation across the broker belong to Phase 1.

## Twenty-four-task trial

Owner-selected sources: Nudge, Researcher and Artifacts. The implementing model wrote all scripts from the discovered schemas and source contracts. These are real application binaries and real MCP HTTP transports, using isolated disposable Postgres/Mongo databases and test identities. No production application records were changed.

- Saved schemas: `docs/trial-schemas.json` (12 Nudge, 33 Researcher, 16 Artifacts tools).
- Model scripts: `docs/trial-tasks.json`, unchanged between attempts.
- First attempt: **22/24 completed**, preserved in `docs/trial-first-attempt.json`. The Researcher inventory fixture had not been inserted and three Artifacts pages had been created with the page endpoint instead of the native document endpoint.
- Corrected-fixture qualification: **24/24 completed**, in `docs/trial-qualification.json`. A host self-check verifies three paginated/fetched issues, three documents, one talk, caught errors and the combined cross-source summary.
- Fresh-run reproduction: the committed fixture script recreated all three test servers and their databases; four representative scripts then verified issue pagination, the inventory record, native document reads and the three-source count summary (`docs/trial-reproduction.json`). Cleanup was verified afterward.
- Tasks exercise filters, summaries, pagination, multi-call dependencies, `asyncio.gather`, caught upstream failures, print output, document manifests/Markdown and cross-source calls. Calls were read-only. Fixture construction is separate from the measured scripts.

`tools/trial-fixtures.py` starts the three actual binaries, creates unique disposable databases/identities, seeds the fixtures, waits for SIGTERM and then removes only its own databases. Its absolute binary locations document this workspace; these are not production source settings. Run it from the repository root. Tokens and server logs stay in ignored `.tools/trial/`, mode 600 for credential files. `tools/trial-seed.js` refuses database names outside the disposable prefix.

## Security and build checks

Read upstream [Rust quickstart](https://github.com/pydantic/monty/blob/main/docs/quickstart/rust.md), [security model](https://github.com/pydantic/monty/blob/main/docs/security.md), [limitations](https://github.com/pydantic/monty/blob/main/docs/limitations/index.md), [snapshots](https://github.com/pydantic/monty/blob/main/docs/snapshots.md) and the pinned crate sources/APIs.

Reviewed all eight [rmcp advisories](https://github.com/modelcontextprotocol/rust-sdk/security/advisories). Their published affected ranges exclude 3.10.0, including GHSA-c9f9-cp8h-jcj5 (<3.9.0), GHSA-5p52-q7pq-9fq2 (≤3.6.0), the tool-schema cache issue (<3.6.0), and the older session, OAuth discovery, redirect and DNS-rebinding issues. `cargo-deny 0.20.2` reports advisories, licenses, dependency bans and sources OK; duplicate transitive versions are warnings, not ignored vulnerabilities.

Verification so far:

- Initial fast tier: 10 cases, 0 failures. Latest fast tier: 11 cases, 0 failures, 2.25 s including the 1.67 s warm build.
- Slow qualification: 12 cases, 0 failures, 5.93 s including the warm build. Slowest: 10,000-run soak 2.557 s; 100 deadlines 2.333 s.
- `cargo clippy --workspace --all-targets`: passed, 43.84 s on its first dependency check.
- `cargo t --touch crates/spike/src/server.rs --budget 5 -p codemode-spike`: **4.16 s**, budget passes. `mold` fixes the slow GNU link. Cold dependency builds still take minutes on this VM.
- Final allocator-matched local qualification: 13 cases, 0 failures. The 10,000-run RSS series grew 320 KiB initially, then remained exactly flat at 10,700 KiB; latest local warm-start check max 3,222 μs.
- Oracle qualification: **13 cases, 0 failures**, 37.10 s including 27.18 s of local build and remote staging. Slowest cases including SSH overhead: soak 3.408 s; 100 deadlines 3.154 s; concurrency 1.249 s. The table uses timings measured inside the Oracle process, excluding SSH.
- Follow-up qualification strengthened the kill test to distinguish automatic pool refill from checking out an existing idle worker, and the recursion test now preserves an already-paused neighbour. All 4 affected worker cases pass on Oracle, 38.29 s including build/staging, and all 11 fast cases pass locally. No production code or thresholds changed.
- `cargo t --manifest-path tools/t/Cargo.toml`: the test runner's argument regression passes, 0.79 s.
- Final `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` and dependency checks pass.

The runner is extended with `T_ZIGBUILD=1` and `T_RUNNER=tools/oracle-test.sh` so tests are still driven through `cargo t`. The runner stages cross-built binaries once and applies a remote kill deadline as well as its local process-group deadline. It does not compile on the VM. Cross-compilation uses cargo-zigbuild 0.24.1, Zig 0.15.2 and glibc 2.35; the target binary has no dependency on newer glibc symbols. The worker SHA-256 is `a708c75b34f04ec8e2ae9e4ccd952d55e75b8380126de129fcb4576b00bf2afb`.

The first remote harness attempt rejected `--test-threads=1` because its argument allowlist omitted `=`; no tests executed in that attempt. The allowlist was fixed before the successful full Oracle qualification. No test threshold was changed.

## Reproduce

```sh
uv venv .tools/monty
uv pip install --python .tools/monty/bin/python pydantic-monty-runtime==1.5.0
cargo t --workspace
CODEMODE_RESULTS_DIR=docs/measurements/local-final T_SLOW=1 cargo t --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo deny check
# With cargo-zigbuild and zig on PATH, and oracle-vm SSH configured:
T_ZIGBUILD=1 T_RUNNER="$PWD/tools/oracle-test.sh" T_SLOW=1 cargo t -p codemode-spike --target aarch64-unknown-linux-gnu.2.35
```

The port selected for the eventual gateway is **2093 on oracle-vm**, subject to a fresh availability check immediately before binding. Caddy and DNS remain the owner's task. No deployment is claimed by this report.
