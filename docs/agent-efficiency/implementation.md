# Agent efficiency proposal implementation

Authoritative scope: https://artifacts.microapps.space/a/npta97dqvf6a,
archived in `proposal.md`. This work is not yet qualified or deployed.

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
