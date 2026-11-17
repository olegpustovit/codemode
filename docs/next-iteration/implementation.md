# Next iteration: proposal v6

Authoritative proposal: https://artifacts.microapps.space/a/7m9atvt6k7xa,
saved verbatim as `proposal.html`. Owner authorized implementation, qualification,
commit, push, redeployment and a new visual results report.

## Scope and completion evidence

- W0: explicit raw MCP, Codex host and Codemode heavy arms; discarded prefix
  warm-ups; blocked arm scheduling; fresh input after the first request; discovery,
  probe and work execute classification. Preserve raw transcripts and failures.
- W1: bounded signature/output-shape manifest, one copy using the E2 placement
  evidence; whole-task execution guidance; independently qualify correctness.
- W2: bounded structural return skeletons and result-shape feedback for Python
  KeyError/TypeError/IndexError, without values; projection example;
  independently qualify correctness and recovery.
- W3: full 40-task, three-repeat standard qualification and E8 heavy packs for
  Codex and Claude, explicitly distinguish client capabilities; compare raw MCP
  and Codex host costs rather than relabeling historical direct results.
- Release: native/hardened/browser qualification, preserved tenant data, existing
  deployment gate, committed/pushed source and verified running binary/public UI.
- Report: new visual artifact with targets, actual results, old/new Codemode and
  correctly labeled controls, source receipts, limitations and research archive.

## Invariants and affected surfaces

Catalogue queries/stub generation, persisted stub refresh and SQLx metadata;
gateway initialize/tool descriptions; execution dispatch/error persistence;
unit/integration fixtures and golden stubs; sibling eval runner, parsers, metrics,
cost checks and their tests; release scripts/receipts; status documentation and
the published report. `/mcp` remains exactly search and execute. Tenant/workspace
scope, blocks, audit, worker isolation, redaction and execution limits remain
enforced. Never grant provider consent or create non-eval production fixtures.

## Initial evidence

- Starting worktree clean at 73aabb4 on `codex/agent-efficiency`.
- Current manifest lists names only, budget 1536 bytes, copied into initialize
  instructions and execute description. Guidance says one execute per step.
- Current brief stubs omit output types; execution already has a bounded
  streaming structural-shape visitor for truncated output.
- Current eval runner interleaves arms and has no discarded warm-up phase.
- Local `codex features list` confirms `code_mode_host` and
  `code_mode_host_output_truncation` stable and enabled. This confirms the current
  client defaults, not what every historical model request saw.

## Candidate changes and checks

- W1 moves the manifest to the execute description. E2's two placements both
  passed 120/120. Description-only reduced median input from 99,345.5 to
  82,881.5 tokens and execute errors from 13.65% to 5.57%. The manifest is
  capped at 3,072 UTF-8 bytes and shows ranked input signatures and declared
  output keys for each visible source. Blocked tools keep their explicit label.
- W2 gives brief search stubs a compact return skeleton and puts a real
  projection example in the execute description. Tools with stored older
  briefs gain the skeleton in the manifest immediately. Brief search stubs
  are rebuilt on existing source synchronization. Shape feedback uses the
  latest successful upstream reply from the same script. Memo/discovery
  payloads are excluded. Only a worker KeyError, TypeError or IndexError
  triggers the streaming structural summary.
- The W2 prompt assembly queries the manifest only for tools/list after
  authentication. Normal tools/call requests avoid fetching the expanded
  catalogue. Both protocol eras still expose exactly search and execute.
- W0 captures numeric input usage from isolated Codex request counter events
  and Claude message usage. Counts must reconcile with the per-run total.
  `fresh_after_first` excludes cache reads and the entire first request and
  includes later cache writes. Warm-ups are recorded separately. Recovery
  counts failed execute calls followed by a successful execute in the same run.
- Three Codex 0.169.0 raw MCP probes failed before any upstream call with
  `code-mode host is disabled`, including explicit direct exposure and disabled
  incremental tools. A matching default-host probe passed. The Claude direct
  probe passed. The failed Codex raw runs have no valid price comparison.
- W1 optimized candidate SHA-256
  `d33df24cae1d87ebcae0a879ae7de7118dba3a46902a5c7c984ab26cd559899c`.
  Its standard qualification is in progress. The first W2 native qualification
  passed 155 cases, including the error response and regression fixture, before
  the manifest lookup moved to tools/list. Requalification covers that change.

## Standard qualification and deployment

W1 source `8a87565` passes all 120 clean gateway runs. W2 source `4845aab`
passes all 120 clean gateway runs and all 114 comparable native Codex-host
runs. The full cohort measurements are in the harness and the compact
verdicts are in `docs/measurements/next-w{1,2}-standard-verdict.json`. The
same classifier recounts E5 at 140/413 discovery-only executes and 91/413
raw-probe executes. Those definitions differ from the original proposal's
regex. V6 scores 109/436 and 65/436. Each of the 11 failed v6 executes was
followed by a successful execute in its run. Per-request input usage
reconciles for all 120 Codemode runs.

The final candidate passes 155 native, 155 hardened ARM, 163 long-tier
hardened ARM and 34 UI tests. The strict browser suite passes 11/12. The
timing assertion measured p95 56.8 ms against 50 ms. A separate functional
run passes the rest of that workflow with the timing assertion excluded.
The disposable candidate passes all 23 L2 checks. Its limited proxy omits
public metadata routes, admin audience handling and the live GitHub source.
The production gate exercises those surfaces for the live build.

Source `4845aabf3bcc45cbad6a7d58daff59c3ed260c28` is on main and deployed
at schema 24. The staged and live ARM binaries match. The restored-backup
preflight preserved 28 tables. L1/L2 regression checks and 8/8 live smoke
tasks pass. All 84 public files match, desktop/mobile sign-in passes, and
the service and backup timer are active. See
`docs/measurements/next-iteration-release.json`. Authenticated live readback
in both MCP eras shows exactly search and execute, a single 2,784-byte
manifest and the projection example. The 2026-era list has private caching
with zero TTL. See `docs/measurements/next-live-mcp-surface.json`.

## E8 raw MCP compatibility

The supported Codex model catalog resolves `gpt-6.1-sol` to
`code_mode_only`, which explains why host-off probes could not dispatch.
Using the supported startup catalog setting with the same model record
and `tool_mode=direct` lets Codex issue native MCP calls with the host off.
Both the warm-up and measured compatibility probe retrieved the correct
code word through MCP. The isolated runner records the catalog SHA.
See `docs/measurements/next-codex-raw-compatibility.json`.

The owner stopped Claude testing. The completed compatibility probe and
captured heavy warm-up remain in the research data. All subsequent model
evals use Codex.

## E8 qualification

Both three-arm packs completed at identical model, client, fixture checksum
and repeat count. Each arm ran in its own block after one excluded warm-up.
Correctness in the order Codemode, Codex host, raw MCP was **30/30/30**
at 1,000 rows and **29/30/15** at 5,000, out of 30 per arm. With all
1,000-row runs clean and correct, Codemode's median API-equivalent cost was
**0.556×** the host's and at most **0.076×** raw MCP's. Both 1,000-row
thresholds pass. The 5,000-row cost comparisons are invalid due to failed
grades, which are preserved. One v6 agent produced an incorrect final
summary after Monty returned all 23 correct payee medians. Raw Nudge agents
reported truncation and did not return verified aggregates. See
`docs/measurements/next-e8-verdicts.json` and
`docs/measurements/next-heavy-answer-check.json`.


## Publication and reproducibility

The [v6 visual report](https://artifacts.microapps.space/a/ny3vep8ck2c7)
version 3 compares Direct MCP through the Codex host, pre-refactor Codemode,
the previous E5 release, and v6 across 38 shared tasks. It shows all six
standard efficiency targets, both heavy scales, model request counts, the
per-task tables, and the observed misses. Its HTML and comparison JSON
match the published files byte for byte.

The [v6 research supplement](https://artifacts.microapps.space/a/zp96zbshmq4s)
version 1 extends the [earlier archive](https://artifacts.microapps.space/a/favtgv57hd8n)
through 2026-11-17T21:52:52.677525+00:00. It contains 564 captured
transcripts and 13 receipt files with 684 rows, including overlapping
recounts and discarded warm-ups. All 10,396 members passed SHA-256
verification after publication and readback, including Git modes for the
source snapshots. The native save confirmation was not exercised.
