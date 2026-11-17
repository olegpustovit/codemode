# Agent efficiency proposal implementation

Authoritative scope: https://artifacts.microapps.space/a/npta97dqvf6a,
archived in `proposal.md`. Implementation is deployed; performance exceptions and evidence are recorded below.

## Required delivery and evidence

- A1: omit optional non-nullable top-level null arguments before validation;
  preserve required, nullable and nested values; deterministic and broker checks.
- A2: validation signatures and three nearest unknown-tool suggestions;
  deterministic checks, with no argument secrets in errors.
- A3: 100 sequential and parallel calls, catchable identical overflow errors;
  real-worker regressions and deterministic sequential-cap check.
- A4: tools/call-only session buckets, token ceiling, 600/min default,
  Retry-After; concurrent eight-job run with zero HTTP 429s.
- B1: sandbox search/help, scoped, audited and budgeted; inline-search check.
- B2: workspace manifest in execute description/instructions, ~1.5 KB budget;
  source discoverability and workspace isolation checks.
- B3: compact input-only brief, top-three full, ten results, deduplicated types;
  size and pagination checks and L4 response-size measurements.
- B4: structural summaries on truncation; bounded UTF-8/result-shape tests.
- B5/C1: schema-detected .all(), max_pages=20, cap/deadline enforcement;
  real pagination and failure checks.
- C2: 64 KB session memo, session expiry, tenant RLS, no credentials;
  persistence, boundary and isolation checks.
- C3: source recipes and execute(scriptId,args), search scripts;
  saved-script binding, authorization and recipe checks.
- C4: _fields projection including .all(); shape/projection checks.
- D1–D3: Tokens setup command, duplicate-connector guidance, six-line playbook;
  rendered UI and MCP description checks.
- D4: explicit codemode/build server identity; initialization check.
- E1–E7: harness flags and requested comparative experiments (40 tasks x 3,
  heavy pack of ten, Claude Code), preserving 100% task/safety success.
- Metrics: median <=3 MCP calls, <=1 pre-execute calls, <=3% execute errors,
  <=80k total and <=15k uncached input, <=18s, heavy >=2x cheaper;
  search median <=3 KB and max <=12 KB. Measure, do not infer achievement.
- Preserve exactly search+execute, workspace dispatch blocks, sandbox isolation,
  tenant credentials and existing data. Run required Rust/UI qualification.
- Commit, merge, deploy through CODEMODE_EVAL_GATE=../codemode-evals
  tools/deploy.sh, then verify public runtime/build and record receipts.

## Current implementation and focused checks

- A1: broker drops only optional top-level nulls rejected by the original
  property's compiled schema. Nullability is computed once during source sync,
  against the original root so references retain their meaning. Required,
  nullable, nested and unknown keys retain their existing validation behavior.
  Unit regression passes; real MCP/Postgres dispatch with `prefix=None` passes.
- A2: validation errors include the input signature; unknown-tool suggestions
  rank the nearest three allowed names in the same source. Monty 1.5 does not
  invoke Python `__getattr__`, so uncaught unknown attributes are enriched at the
  gateway error boundary instead. Direct `_call` unknown-tool errors remain
  catchable. Real MCP/Postgres error-guidance regression passes. This behavior
  still needs external deterministic eval coverage and cross-workspace tests.
- A3: suspension allowance is 216; valid overflow callbacks receive the same
  Python RuntimeError (`ToolError` alias) before dispatch. Real-worker regression
  passes for 101 sequential calls and `gather` overflow (0.040 s).
- D4: explicit `codemode` server identity with build ID (package version for
  development); real protocol initialization assertion passes.

Focused checks used `cargo t`; optional-null unit check took 0.010 s, edit-distance
unit check 0.010 s, and the real upstream/broker check 0.282 s after the final
fixture change. Initial dependency build took 266.16 s; subsequent focused builds
were 0.15–5.56 s. Formatting and diff-whitespace checks pass.

Workspace qualification on an owned disposable PostgreSQL cluster with two
verification CPUs ran 131 cases: 130 passed, and the new identity assertion failed
because the test indexed an already-unwrapped RPC result. The assertion was fixed;
the complete dual-era protocol/workspace workflow then passed (0.484 s, 3.58 s
including build). The initial workspace build took 159.24 s; the suite took
169.69 s total. No production state has changed. A4, B1–B5, C2–C4,
D1–D3, external experiments, metric gates, final qualification, merge and deploy
remain outstanding. The root MCP surface remains `search` and `execute`.

A2 remaining fidelity check: Monty's caught `AttributeError` still has its original
message; gateway enrichment currently applies only to uncaught errors. Investigate
the SDK's host-backed class `NameLookup` support before marking A2 complete. Do
not replace the requirement with an eval that only checks uncaught failures.

## Discovery, paging, projection and admission checkpoint

Implemented locally, pending external comparison and deployment:

- A4: only `tools/call` consumes the MCP rate budget. Session admission uses the
  same inferred `agent_sessions.id` as execution history, including grant/refresh
  continuity. Defaults are 600 calls/minute/session with token and workspace
  ceilings of 6000. All three buckets admit atomically; HTTP 429 retains
  `Retry-After`. Initialization and listing remain available after exhaustion.
  Existing explicit environment overrides still apply and must be inspected at
  rollout. Read request bodies are bounded at 1 MiB, above the escaped 64 KiB
  Python source limit. Identity's separate HTTP rate limit retains its old values.
- B1: `await tools.search(query)` returns brief matches inside an execution.
  The reserved broker path `@codemode/search` is workspace-scoped, audited and
  charged to the common 100-call limit. `help(tool)` synchronously returns the
  generated full stub already loaded for this execution, without a broker call.
  Full help is withheld for tools outside workspace tool scope. A source named
  `search` can coexist with the inline search capability.
- B3: ten-hit pages, full stubs for the first three hits only, compact input-only
  brief signatures, and repeated exact TypedDict/import definitions emitted once
  per full page. The 61-tool golden corpus was intentionally regenerated and
  offline SQL metadata refreshed on disposable PostgreSQL. Search response size
  targets have not yet been measured or enforced as L4 gates.
- Blocked-tool visibility: in-scope blocked tools now carry `blocked: true` in
  search. Out-of-scope tools stay absent and broker blocks remain enforced.
- B4: truncated result strings append a structural summary with object keys,
  array counts and the first item's shape. A streaming visitor bounds retained
  shape depth, nodes, keys and fields; it never constructs a full JSON result
  tree. Preview plus summary obeys the same escaped JSON byte budget, and dropped
  bytes still count omitted original bytes rather than added summary text.
- B5/C1: unambiguous cursor/nextCursor or next_cursor schemas gain `.all()`.
  Nested envelopes and local schema references are supported. Every page remains
  an ordinary broker call. Defaults to 20 pages, accepts 1–100, and raises instead
  of silently returning an incomplete total when the limit is reached. Repeated
  cursors and malformed envelopes fail precisely. The shared call cap and wall
  deadline remain in force.
- C4: `_fields` trims rows inside the worker, including `.all()` results. Invalid
  options fail before upstream dispatch. Ordinary list responses retain their
  envelopes and cursors. No dependencies were added.
- Documentation: corrected the smoke receipt from three tools to two and
  documented existing `execute(scriptId)` plus new rate defaults.

Real-worker tests cover paging, projection, aliases, page/cursor limits, early
option validation and exhausting the common call budget partway through `.all()`.
The real MCP/Postgres test covers inline discovery, help, auditing, a 100-search
cap, an empty workspace and a connected source named `search`. Policy tests cover
blocked visibility, dispatch denial and withholding out-of-scope help.

A 133-case workspace checkpoint passed before the later shape/admission changes.
The next 137-case run had 136 passes and exposed that the shape depth bound cut
primitive field types too early; the bound was corrected while preserving the
32-node cap. The latest complete qualification result will be recorded below.

Remaining original scope: caught unknown-attribute guidance (A2), the workspace
manifest (B2), session memo (C2), recipes and script arguments (C3), Tokens setup
and Codex instructions/playbook (D1–D3), all requested experiments and metric
gates, final native/Oracle/UI/browser qualification, merge and gated deployment.
The root `/mcp` surface still has exactly `search` and `execute`.

Latest checkpoint: **137/137 native Rust checks pass**, on owned PostgreSQL 18
with two verification CPUs; 24.91 s total including 14.55 s build. Slowest case
2.329 s. `cargo fmt --check` and `git diff --check` pass. Clippy completes in
8.32 s with the repository's warning set (including test `unwrap` warnings),
not a warning-free result. No UI, Oracle or external model comparisons have
been claimed for this checkpoint, and no production changes have been made.
The existing stub generator example remains the way to regenerate the golden;
there is no test bypass or new golden-update option.

## Manifest, script arguments and session memo checkpoint

- B2: initialization instructions and `execute` descriptions contain a dynamic
  workspace manifest, bounded to 1536 UTF-8 bytes. It names sources, tool counts,
  representative Python names and blocked tools. The SQL explicitly binds source
  and workspace tenants and honors source/tool scopes. Personalized tool lists
  use private, immediately stale cache hints in the 2026 MCP era.
- C2: migration 0023 adds `agent_session_memo`, a JSON object capped at 64 KiB,
  keyed by the actual inferred agent session. `memo.get(key, default)` and
  `memo.set(key, value)` are synchronous broker capabilities. They remain audited,
  count toward the 100-call budget, and commit each write, including before a
  later execution failure. Concurrent writes merge atomically. Session expiry
  and deletion cascade to memo rows through the existing retention sweep.
  RLS rejects foreign reads, updates and inserts. No host credentials enter the
  worker; memo additionally rejects sensitive keys and common credential formats.
  Memo audit arguments and previews withhold submitted contents.
- C3 (binding portion): `execute(scriptId, args)` binds an optional JSON object as
  Python `args`, without inserting it into Python source or the audited script
  text. Arguments have their own 64 KiB limit. Normal `code` calls reject `args`.
  Existing script authorization, policy and audit behavior is preserved. Source
  recipe scripts are still outstanding.
- D2/D3: the six-line execute contract promotes inline discovery, help, gather,
  pagination, field projection, non-sensitive session memo, concise results and
  catchable errors. Initialization instructions recommend `features.apps=false`
  or disconnecting duplicate connectors when they overlap Codemode.

The sandbox now validates a synchronous broker callback alongside async calls;
real-worker checks prove that a synchronous helper preserves already-pending
async futures and that its errors remain catchable. A separate real-worker probe
proves host-backed source objects can retain Python callables while reporting
lazy missing attributes. The A2 production fix still needs the callable source
named `search` case handled before replacing existing source wrappers.

**144/144 native Rust checks pass**, on an owned disposable PostgreSQL 18 cluster
with two verification CPUs: 42.15 s total including 31.13 s build, slowest 2.327 s.
The new memo behavior and RLS cases each take 0.303 s in focused qualification.
The argument-binding test verifies code-like strings stay data, do not dispatch
upstream calls, and respect the independent argument limit. Dual-era HTTP checks
verify manifest scoping, playbook discovery, source names and private cache hints.
Offline SQL metadata was refreshed through ordinary candidate startup migrations
on the disposable cluster; production was not touched.

Still required: complete caught unknown-tool guidance, source recipes, Tokens
setup guidance, E1–E7 and the metric gates, final Oracle/UI/browser qualification,
merge and gated deployment. Native checks are not external model performance
acceptance. Existing incremental build-budget limits remain visible.

Current checkpoint Clippy completes in 7.40 s with warnings, including the
existing test-unwrap warning policy; formatting and diff-whitespace checks pass.
The SDK probe also established that host class types do not permit adding a new
`__call__` class attribute from Python. A2 must use a supported host callable
mechanism for the source named `search`, not that unsupported assignment.

## Recipes, setup guidance and complete unknown-tool errors checkpoint

- A2: source namespaces now use supported host-backed SDK objects. Both missing
  method calls and bare attribute lookups become scoped, audited broker errors
  with up to three nearest reachable tool names; Python can catch them as
  `ToolError`. Root namespace typos and a source literally named `search` are
  covered. Native source-call routing validates the host object UUID and query
  shape. Only the inline-search result uses native JSON conversion; ordinary
  upstream results keep their raw JSON pass-through path. The previous uncaught
  AttributeError text adapter is removed.
- A3: the suspension allowance includes initialization of all 64 supported
  sources plus the root namespace, without reducing the 100-call budget. Worker
  callback identities, pending futures and synchronous/name-lookup replies stay
  validated. A real-worker regression exercises all 65 namespace objects followed
  by 100 successful sequential calls.
- B5: live eval-only provider metadata exposed integer Moneybag cursors, declared
  date/amount sort companions and optional terminal cursors. Pagination now
  follows those contracts and retains strict errors for missing required cursor
  fields. The real-worker regression checks an integer cursor, its sort companion
  and an omitted final cursor. The schema reads were read-only.
- B2: manifest samples prioritize the proposal's useful discovery entry points
  (such as list_issues and entries_list), rather than allowing alphabetical create
  methods to crowd them out. A database regression verifies this within the
  unchanged 1536-byte manifest budget.
- C3: supported canonical Nudge and Moneybag sources receive workspace-scoped
  starter scripts on script discovery/listing. Installation requires the expected
  tool/schema contracts and honors tool scope and the existing 128-script quota.
  Migration 0024 records installation, so edits remain edited, deletions remain
  deleted, and matching custom scripts are never overwritten. New receipts are
  RLS-protected and cascade with source/workspace removal.
- The Nudge recipe resolves a team key and ordinary status name, validates its
  arguments, then makes exactly one create call. The Moneybag recipe groups
  matching-account postings by month and currency with exact integer/scale decimal
  arithmetic, excluding balancing and similarly prefixed unrelated accounts.
  Both use ordinary paginated broker calls and the existing execution limits.
- D1: the one-time token receipt includes a verified `codex mcp add` command using
  CODEMODE_TOKEN, the matching /mcp or /mcp-admin URL and server name, approval
  configuration instructions, and duplicate-connector guidance. Token values are
  not inserted into commands. Existing UI components and code-panel styles are
  reused; access selection is disabled while creation is pending.

The **152-case native workspace suite passes**, 45.81 s including 33.68 s build;
slowest case 2.342 s. The subsequent manifest-priority regression also passes
(0.192 s). The four focused recipe cases pass, including worker execution and
PostgreSQL idempotence, edit/deletion preservation, quota and tenant/RLS checks.
Current Clippy completes in 8.47 s with the repository warning set; formatting and
whitespace checks pass. Offline SQL metadata was regenerated on disposable
PostgreSQL through ordinary startup migrations.

Frontend typecheck and lint pass (existing three shared-component warnings),
**34/34 Vitest tests pass**, and two focused actual-binary Chromium workflows pass
in 42.0 s, including token receipt setup, mobile overflow and separate management
resource authorization. The staged UI fingerprint is **4fe138fb8eae**, with initial
JS 160473 B and CSS 15757 B gzip, lazy chunks within 100 KB. These browser checks
preceded the final source-namespace adapter; they are a UI checkpoint, not final
combined-artifact acceptance.

All named application features now have local implementation and regression
coverage. Still required before claiming delivery: the requested E1–E7 experiments
and metric gates; final native/slow/Oracle/hardened/UI/optimized-browser checks;
merge; gated deployment and public runtime verification. Production remains
unchanged. Functional tests do not prove the token/time targets.

## Release qualification and first measured model checkpoint

- The 153-case native suite passes with normal hard deadlines: 20.11 s total,
  4.03 s build, slowest case 2.361 s. The complete 161-case native slow suite
  passes in 89.18 s including 2.96 s build.
- The complete **161-case hardened Oracle slow suite passes**, 166.75 s including
  90.96 s staging/build. It includes 10,000-run memory stability, worker limits,
  access revocation, upstream stream cancellation and source removal/outage.
  Two pre-existing test helpers ignored CODEMODE_MONTY_BIN; they now honor the
  same isolated-worker override as the surrounding tests. An earlier strict
  Oracle checkpoint had 152 passes and that helper-path failure; its focused
  corrected test passes. A final strict Oracle run is still in progress.
- The optimized browser run exposed a real empty-tool-scope failure: PostgreSQL
  correctly returned a null path from the manifest's LEFT JOIN, but SQLx's
  inferred nullability rejected it. The query now explicitly marks path nullable.
  Regression coverage includes an unsynced empty source and a source whose tools
  are all excluded. Both OpenAPI and GraphQL browser workflows now pass.
- After this correction, **11 of 12 optimized browser workflows pass**. The
  integration workflow's pane click-to-paint p95 measured 56.4 ms against its
  unchanged <50 ms gate during concurrent qualification/model work. It must be
  rechecked under quiet conditions; no timing threshold has been relaxed.
- All **23 deterministic eval checks pass** against an optimized disposable
  candidate, using only the three eval app accounts. This includes real optional
  null arguments, caught signature/suggestion errors, 100 sequential calls,
  inline search/help, pagination/projection, memo persistence/bounds, sandbox
  isolation, audit tenancy and the 60 s deadline. Receipt:
  `docs/measurements/agent-efficiency-deterministic.json`.
- The harness now records pre-execute calls, execute errors and individual UTF-8
  search response sizes. Its strict efficiency checker rejects missing data and
  failed/unclean runs. E2 manifest-placement and E3 playbook flags are implemented
  through a disposable proxy; JSON/SSE self-checks and all six actual MCP surface
  combinations pass. Harness commit: `b1675b2` in ../codemode-evals (local repo).

The first eight-job Codex run passes **8/8 tasks, zero execute errors, zero HTTP
429s**. It does **not** meet every efficiency target: medians are 5 calls, 1.5
pre-execute calls, 96,279 input tokens, 13,609 uncached input tokens and 24.7 s.
Search median is 1508 B and maximum 12,805 B. This is an early smoke measurement,
not the 40-task comparison. Receipt: `agent-efficiency-eight-jobs.json`. Brief
signatures subsequently stopped duplicating the separate description field;
complete full stubs/help and pagination notes are preserved, and the golden
corpus/error assertions were updated. A full 234-run, three-repeat comparison
with eight jobs and both arms is currently running on that corrected candidate.

Remaining: complete E1–E7 coverage, all required metric gates, quiet pane timing
acceptance, final checks, merge, gated deployment and public verification. The
application is not declared delivered; production remains schema 22. Builds
still exceed the historical incremental latency targets (optimized rebuild
186 s, initial optimized build 330 s); no build-budget pass is claimed.

## Full comparison: failed acceptance, with corrected error accounting

Final strict hardened Oracle qualification passes **153/153** under normal hard
deadlines (292.77 s total, 255.98 s build/staging). The optimized browser pane
check was repeated after the full model run: p95 **52.9 ms**, still above the
unchanged <50 ms gate. The other eleven workflows pass; the pane gate remains
unresolved and is not waived.

The full default-client comparison finished: **234 runs**, three repeats, eight
jobs, 233/234 task successes, zero HTTP 429s and zero contaminated runs. The one
task failure is direct MCP's third `nudge-team-by-name` run, which declined to
use a listed team. Six account-cleanup calls also failed because the old harness
passed a date to Moneybag's boolean `close` argument. The harness now uses true;
all six specifically identified eval-run accounts were subsequently closed.
Original task/teardown receipts are preserved, not rewritten as successes.

A critical measurement correction: Codex JSONL's result omits MCP isError even
when the gateway reports a failed execution. The old harness therefore counted
zero execute errors incorrectly. It now reads the explicit execution status,
with four unit checks covering dropped flags, payload fields and call/byte
accounting. Saved transcripts were recounted into a new receipt without
rerunning models or graders. The early eight-run result still has zero errors;
the full comparison has **42/348 failed executes (12.1%)**.

Full corrected medians: **5 MCP calls**, **1.5 pre-execute calls**, **96,095.5 total
input tokens**, **10,656.5 uncached input tokens**, **26.4 seconds**. Search median
is **1463 B**, maximum **9657 B**, so removing duplicate brief descriptions fixes
the measured search-size gate. The call, pre-execute, execute-error, total-input
and time gates still fail. Errors include storing oversized memo datasets,
artifact ID lookup mistakes, `next(list)` misuse, provider help-topic mistakes
and unsupported arguments. No delivery or performance success is claimed.

E5 investigation: codex-cli 0.169.0 lists the old tool_search switches as removed.
The current documented omit_tools_from control was probed for initial-only and
deferred-only exposure. Initial-only finds Codemode; deferred-only exposes no
usable tool search in this client, including a separate suggestion-enabled
probe. These are explicitly failed compatibility probes, not a completed valid
40-task E5 experiment. E1, E4, E6, E7 and full E2/E3 measurements also remain.
See `docs/measurements/agent-efficiency-full-comparison.json`; the application
remains unmerged and undeployed.

Pane diagnostic checkpoint: the benchmark now records click-handler and
layout-ready time separately, while retaining the original next-frame paint
measurement and unchanged thresholds. In an instrumented optimized run during
E2, p95 is 6.8 ms in the handler, 9.6 ms until layout is ready, and 55.7 ms until
the following frame. This locates most of the delay after layout; it does not
prove a shader cause or waive the gate. Receipt:
`docs/measurements/agent-efficiency-pane-profile.json`. No application UI or
shader behavior was changed. E2 instructions/description and E3 playbook-off
measurements are running sequentially, 40 tasks x3 each, with source binary
checksums and client/harness versions recorded in new receipts.

## Placement/playbook comparisons and measured corrective guidance

Each variant ran 40 tasks x3 against the same frozen optimized binary:

| Variant | Task passes | Median calls | Pre-execute calls | Execute errors | Input tokens | Seconds |
|---|---:|---:|---:|---:|---:|---:|
| E2 instructions-only manifest | 120/120 | 5 | 1.5 | 13.65% | 99,345.5 | 25.85 |
| E2 description-only manifest | 120/120 | 5 | 0 | 5.57% | 82,881.5 | 24.7 |
| E3 playbook off | 119/120 | 5 | 2 | 6.45% | 93,481 | 25.3 |

No variant meets all release gates. The instructions-only variant exposed a
14,791-byte full `entries` search result. Discovery descriptions now have a
180-byte UTF-8 summary limit with an explicit ellipsis, while full stubs and
help retain their complete docs. The actual same query now returns **11,838 B**,
with ten hits and exactly three full stubs. The new summary regression verifies
UTF-8 and zero additional allocations; the existing page allocation budget holds.

The follow-up keeps the required manifest in both locations but explicitly says
its Python names are already loaded inside execute. Search/playbook guidance
clarifies single-name or OR queries, tiny non-sensitive memo values (64 KiB),
using real returned IDs, and avoiding generators/next(list). The memo-limit error
now suggests IDs or small summaries. These changes respond to observed failed
transcripts; their model-performance effect has not yet been measured.

**154/154 native checks pass**, 17.20 s including 5.31 s build. SQL OR/conjunction
behavior is covered by the scoped catalogue regression. Oracle/optimized/model
acceptance for this follow-up is still outstanding. The optimized executable
used for E2/E3 and the running E4 comparison was deliberately left frozen.

E4's first run was invalid: the proxy forwarded lowercase content-length plus a
regenerated Content-Length. It was stopped and its HTTP-400 evidence preserved.
The proxy now delegates framing to http.client after stripping incoming framing
headers. A real Codex probe and lowercase-header/rewritten-UTF-8 probes pass;
the corrected 120-run E4 comparison is running. This is a harness correction,
not a gateway or safety failure. E5 legacy-client compatibility is still being
probed; E1, heavy-pack E6 and Claude E7 remain unfinished. No merge/deploy.

## Corrected E4 and memo-guidance prototype

The corrected E4 inline-off comparison completes **120/120 task successes**.
Median calls remain 5, pre-execute calls 2, execute errors 15.51%, total input
99,105.5, uncached input 11,245 and time 25.4 s. The 14,791 B search case belongs
to the frozen pre-summary-correction binary; the old executable is preserved at
ignored `target/agent-efficiency-frozen/codemode`, checksum-bound to its receipt.

An eight-task debug prototype of the new guidance improves calls to 3.5 and
input to 69,236 tokens, but its two execute errors exposed a documentation
regression: the revised memo sentence omitted get/set, and agents attempted
dictionary assignment. Explicit `memo.get(key)` / `memo.set(key, value)` guidance
is restored, including that indexing is unsupported. The next eight-task debug
prototype passes **8/8 with zero execute errors**, medians **3 calls**, **0
pre-execute calls**, **60,129 input**, **10,274.5 uncached input**, **20.05 s**.
Only the time gate fails in that small sample. These are debugging measurements
concurrent with the old E4 cohort, not final optimized or full L4 acceptance.
The optimized follow-up will run without overlapping model cohorts.

E5 pinned-client probes are preserved as failures. Official CLI 0.134.0 retains
the older always-defer switch, but cannot decode the current model catalogue's
max reasoning level; selected model requests are rejected for that client. No
valid on/off experiment is claimed. No global client or provider consent was
changed. E1, valid E5, the ten-task heavy pack/cost gate and Claude E7 remain, as
do the full optimized comparison and pane timing gate.

Latest native verification after restoring the memo API wording: **154/154 pass**,
15.52 s total including 3.62 s build. The application remains unmerged/undeployed.

Optimized follow-up smoke (source bcc3736, build 217 s, no overlapping model
cohort): **8/8 pass**, zero execute errors, median **3 calls**, **0 pre-execute**,
**59,584.5 input**, **9377 uncached input**, **19.45 s**. Only time exceeds its
18 s gate in this small sample. The full updated 234-run comparison is now
running, with the eval dataset held unchanged. Receipt:
`docs/measurements/agent-efficiency-guidance-optimized-smoke.json`.

## Full guidance comparison and heavy-data probe

The optimized full comparison completed 232/234 task successes: Codemode 120/120,
with two direct-arm failures retained in the receipt. Codemode medians are 3
calls, 0 pre-execute calls, 68,057.5 input tokens, 10,169.5 uncached tokens and
20.2 s. Execute errors are 13/414 (3.1401%). The error and time gates still fail.
Parameter descriptions are now included in in-sandbox full help, and the contract
explicitly excludes unsupported asyncio.gather keyword arguments. Native checks
after this correction: 154/154 pass. Updated model qualification remains due.

Both heavy datasets contain 1,000 verified records; repeat seeding preserved
their semantic fingerprints. The four-run heavy probe passes both arms for one
Nudge and one Moneybag task. This is not the full E6 or its cost gate. The Claude
adapter passed an actual isolated MCP probe and its parser/configuration checks;
the full E7 comparison has been started. No release merge or deployment yet.

Qualification continuation: the two direct-arm erroneous comments from run
78cabb were matched by exact comment ID, issue, body and eval author, then deleted
using current update timestamps and audited eval cleanup commands (HTTP 200).
The eval harness now paginates comment graders and removes only current-run
comments owned by its service account; ten parser/configuration/cleanup checks
pass. Historical receipts are unchanged. The running Claude cohort loaded the
previous harness at startup, so it retains its original grading and cleanup.

The final help-correction commit is 22d4acf. Its optimized build and hardened
ARM test run are in progress. A separate b779625-based E1 worktree has been
created for the A1+A2 counterfactual; it is not qualified or part of the release.

## Final help-correction isolation checkpoint

Source 22d4acf passes the strict hardened Oracle suite: 154/154, build/staging
183.43 s, total 219.85 s, slowest 3.999 s under its 5 s integration deadline.
The hardened slow suite also passes 162/162: build/staging 172.18 s, total
247.24 s. These checks include the real-worker parameter-help assertion.
The optimized host build completes in 245 s; build budgets remain exceeded.

The additional current-Codex E5 probe enabled code_mode, code_mode_tool_search
and tool_suggest together with deferred-only MCP exposure. It still could not
access the eval artifact and is retained as a failed compatibility probe, not
a completed E5 experiment. No release qualification waiver is implied.

## E7 first full cohort and E1 isolation

The first Claude cohort completes all 234 runs. Codemode tasks pass 120/120.
The raw direct grades are 111/114: all three failures are the payee grader's
substring false negative. Their answers correctly name Bulk Books/Cafe/Metro
and explicitly exclude Eval Market, but the grader rejected any mention of
that excluded name. The original receipt and a distinct trace-only recount are
preserved; no old task grade has been changed. Future cohorts request an exact
JSON payee array and compare exact names, with regression checks.

The recount separates native client ToolSearch from actual MCP invocations,
retaining both total calls and client calls. Pre-execute discovery remains
conservative and counts all tools. Claude Codemode medians: 3 actual MCP calls,
4 total tool calls, 1 pre-execute call, 27,250 input tokens, 5,192 uncached tokens
and 12.3 s. Execute errors are 5.56%, still above the target. The twelve harness
checks pass. E7 results demonstrate lower latency/input, not full release gate
acceptance. The full 60-run heavy comparison is running on source 22d4acf.

E1 lives separately on branch codex/agent-efficiency-e1, commit c99ee9c, based
on b779625/schema 22. It includes A1 and the A2 error-feedback/host-namespace
backport without exposing search/help/pagination/memo/projection helpers.
The baseline user suspension allowance is retained (only trusted namespace
construction is compensated); its real upstream regression still fails after
55 audited calls, rather than accidentally incorporating A3. All 104 native
core checks pass. Its optimized experiment build is in progress and no E1
model result is claimed yet. The counterfactual is not a release branch.

## E6 full heavy cohort

All 60 heavy runs completed: Codemode 29/30, direct 30/30, zero 429s. The
Codemode failure computed integer cents correctly but its final answer converted
them to euros despite the requested cents; its trace and failed grade remain.
Total uncached input is 271,900 versus 685,512 (2.52x fewer), cached input
1,635,456 versus 2,167,808, output 19,232 versus 21,233. Codex supplies no USD
cost. The new cost checker requires exact ten-task/three-repeat/both-arm
coverage and clean outcomes. It uses client-reported USD when available;
otherwise it proves savings only from a conservative bound over all nonnegative
billable token prices, without inventing a price book. Its bound here is 0.90576,
so 2x monetary savings are unproven and the gate fails. Fifteen harness checks
pass, including missing coverage, invalid accounting and cost cases. No raw
receipt was regraded and no failed model task was silently retried.

The E1 optimized counterfactual built in 233 s, checksum
d2638670cb64e541ce7c084d9dcfb032b7e552d8763369c30f507dc1267107c8.
Its actual MCP probe passes. The full 120-run E1 cohort is running at two jobs
to respect the baseline admission limit; final release experiments still use
the preserved source-22d4acf optimized binary (SHA-256
95a90c44faea63892cb5b5ab47889f4bdadefa6216e58b956566489d034e4ea1).
The E1 binary embeds the copied historical UI staging and warns it is stale;
this is an MCP-only counterfactual, never a deployable UI release artifact.

## Current-client E5 compatibility and browser checkpoint

The current-client deferral problem is resolved at probe level: earlier probes
omitted code_mode exposure as well as initial/deferred tools. Keeping code_mode
exposure, enabling code_mode and tool_suggest in both treatments, and toggling
code_mode_tool_search plus the corresponding initial/deferred exposure yields
two passing actual MCP probes (16.6 s on, 14.3 s off). The full E5 comparison
is now running as interleaved matched treatments: 40 tasks x 3 repeats x 2, with
order alternating per task/repeat under the same load. Every row records its
treatment. The prior failed probes remain historical, not successful cohorts.

A reused on-probe output filename briefly replaced the historical working-tree
receipt; the original was restored from Git and the new receipt renamed. The
runner now rejects existing output paths before starting work and writes with
exclusive creation. The deployment gate's temporary output path was updated to
match. All seventeen harness checks pass. No historical grade was changed.

The source-22d4acf optimized browser run completes 11/12 workflows. Integration
pane timing fails its unchanged 50 ms gate: click-to-paint p95 74.9 ms, handler
8.9 ms, layout-ready 12.7 ms. The approved shader/UI has not been altered to
hide the failure. This run overlapped E1 and small compatibility probes; it is
not evidence of idle hardware acceptance. The structured timings are preserved
in agent-efficiency-pane-final.json. No merge/deployment acceptance is claimed.

## Passing E5 configuration and release setup guidance

The full interleaved E5 cohort completes 240/240 correct. Deferral off passes
all standard targets over its 120 runs: median 3 MCP calls, 0 pre-execute calls,
2.6634% execute errors, 54,003 input, 13,755 uncached input and 16.95 s. Deferral
on also passes 120/120 tasks but misses uncached input (15,181) and time (20.45 s).
The execution-token receipt now supplies the measured Codex 0.169.0 gateway-only
configuration: per-server deferred exposure omitted, code_mode/tool_suggest on,
code_mode_tool_search off. Management-token guidance retains its default profile
because E5 qualified only the two-tool execution resource. Existing duplicate
connector advice remains. UI typecheck, 34 Vitest checks, lint and staging pass
(three incumbent lint warnings); staged fingerprint 5dd5ac2e52f0, initial JS
160472 B and CSS 15757 B gzip. Combined optimized-browser verification is next.

Cached Chromium 156 also misses pane timing: p95 73.4 ms, handler 7.8 ms, layout
11.1 ms. This rules out a browser-version change alone as the fix. Graphics
metadata collection originally raced React mounting; it now waits for the canvas
after all timing samples, so it cannot warm up or alter the measured clicks.
No shader cause or hardware acceptance is claimed.

Official GPT-6.1-Sol rates were verified from OpenAI's model documentation on
2026-11-17. The cost checker now accepts a named-model Standard API-equivalent
estimate, proves short-context eligibility conservatively from per-run totals,
and refuses cache-write estimates without TTL evidence. A fresh explicit-model
60-run heavy cohort passes all tasks but estimates $0.9921356 versus $1.7281208
(1.74x, below the 2x gate). Original default-model receipts remain unpriced.
A new comparison is running with the qualified gateway setup against the direct
arm's unchanged native defaults; this distinction is explicit in metadata.

## Research publication and final evaluation checkpoint

The owner requested all captured evaluation runs for a future paper. Private
research artifact: https://artifacts.microapps.space/a/favtgv57hd8n (version 1).
Its 2026-11-17T18:18:27Z snapshot includes 2,623 CLI transcript files, 49 receipts,
3,158 receipt rows (overlapping originals/recounts, not independent attempts),
historical harness/gateway source, qualification logs and per-member hashes.
Readback verifies all 11,110 archived member hashes. All raw results files needed
zero credential redactions; credential-like literals in selected logs/source
were masked. Download confirmation was not exercised; the complete files were
retrieved through the artifact API and verified. The platform accepts encoded
ZIP parts as text, with checksum-verified save controls and a local restore
script. Updates will preserve the artifact ID and distinguish snapshot versions.

The final source-9f36b2d Claude cohort is complete: 234/234 correct, all applicable
standard gates pass. Codemode medians: 3 MCP calls, 1 pre-execute call, 2.1807%
execute errors, 27,095 input tokens, 5,405.5 uncached input and 11.45 s. E1 is
also complete: 120/120 correct but median 6 calls, 2.5 pre-execute, 8.55% execute
errors, 108,711.5 input and 25.5 s. This isolates why A1+A2 alone are insufficient.

The declared 5,000-row pack passed all 60 tasks, with the original 1,000-row
fixture hashes preserved. Its estimated median API-equivalent cost is $0.0479716
for Codemode versus $0.0587742 direct (1.225x); it fails the 2x cost gate. The
negative result is retained. No data-size or threshold waiver is implied.

Draft PR #4 contains the implementation and explicit open gates. No merge or
deployment has occurred. A production-UI identity check verifies all 84 staged
historical frontend files are byte-identical to public HTTPS. The attempted
local historical-UI timing run stopped at the stale-bundle guard before any
measurement; no baseline timing is claimed from its leftover output file.

## Owner-directed release decision

On 2026-11-17 the owner explicitly requested proceeding with deployment after
being shown the remaining qualification limits ("dude pls get this deployed i
think all is good"). Proceed with the existing correctness/safety deployment
smoke gate and rollback, retaining the unmet 2x heavy-cost and 50 ms pane timing
results as release limitations. This is an owner-directed release decision, not
an assertion that those performance targets pass. The in-progress minimal-profile
experiment remains research and does not change the qualified product profile.
No shader changes are included. A proposed, uncommitted expansion of the gate
was removed; the existing smoke gate remains unchanged.

Research archive version 2 now contains 2,638 transcript files, 51 receipts and
11,513 verified members. Receipt rows overlap and are not unique attempts.

## Deployment completion

PR #4 merged as 8d0aae27091e8e7c88e5ca02011b920fed51f1b4. The exact merged
source was built for ARM in 5m31s and deployed through the existing live eval
gate. All 26 existing tables survived restored-backup migration; schema 24,
public/local build and running checksum match. All eight live smoke tasks pass,
including both safety tasks; public 84-file byte comparison, security/deep-link
smoke and desktop/mobile sign-in checks pass. Service and backup timer are active
with NoNewPrivileges and PrivateTmp. No provider consent was granted.

The final minimal-profile research cohort is 60/60 correct but still fails the
2x heavy-cost target (median ratio 0.6145, about 1.627x saving). It does not change
the released profile. Original negative results remain available.

Final report: https://artifacts.microapps.space/a/ymj65gs0sx03 (Markdown readback matches).
Research archive version 3 includes 2707 transcript files, 55 receipts and 12862 verified archived members, including all captured release smoke attempts. Receipt rows overlap; no unique-run count is inferred.
