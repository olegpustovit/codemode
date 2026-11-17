Codemode MCP gateway · eval results · 17 Nov 2026

# Code mode works and is safe. *It saves tokens on data-heavy work*, but it's slower, and two bugs need fixing first.

Codex ran 40 real tasks across artifacts, moneybag and nudge, 3 times each. In one setup it went through codemode; in the other it connected to the apps directly. Graders checked the apps' real data, never the agent's own claim of success. Release `2f23413` shipped through the new eval gate.

✓ 234 / 234 runs pass ✓ 0 safety violations ✕ 55-call cliff ✕ 429s kill agents ▲ 50% slower

[Issues](#issues)[Numbers](#numbers)[Plan to beat raw MCP](#plan)[Checks](#checks-h)

100%

pass rate, both setups (pass^3 on 38/38 shared tasks)

−13%

uncached input tokens vs direct (2.13M vs 2.46M)

+50%

median time per task (28.6 s vs 19.1 s)

6 vs 4

median MCP calls per task (search → execute loop)

30/33

protocol, sandbox and broker checks pass

2

real bugs: sequential call cap and rate limiting

## Issues: what's broken and where

Ranked by how much each one hurts agents in practice. Every item names the code, the evidence and the fix, and has an eval check that confirms when it's fixed.

P0 · BUGmonty-pool / execution limits

### Sequential tool calls die at 55, not 100

`for _ in range(101): await tools.x()` fails at call **55** with `RuntimeError: suspension limit 110 exceeded`, and `try/except` can't catch it. Each awaited call costs about 2 Monty suspensions, so sequential code gets half the documented budget.

**Fix:** set the Monty suspension limit to at least 2 × the call cap plus headroom, and raise one catchable error: "tool-call limit (100) reached".  
check: l2.call-cap-sequential

P0 · BUGgateway/src/config.rs:78 · server.rs:461

### 60 requests per minute per token makes agents fail hard

With 6 parallel Codex runs on one token, codemode returned **HTTP 429**. Codex doesn't retry: **12 tool calls failed**, and in **4 of 39 runs** the model saw no tools at all, because startup `tools/list` was throttled. Pass rate fell from 100% to 92%. `initialize` and `tools/list` count against the same budget.

**Fix:** raise the default (60/min is about one busy agent), count only `tools/call`, and bucket per agent session instead of per token.  
check: run.py http\_429 == 0 at 6 jobs

P1 · COSTcatalogue/stubs · search

### The search → execute round trip is a time tax

Median **6 MCP calls vs 4** (about 4 searches plus about 3 executes per task). `detail:"full"` returns large `TypedDict` signatures: **20 KB vs 10 KB** of tool results. Nudge tasks cost about **2×** direct, because ID resolution needs several searches.

**Fix:** make brief signatures compact (no output TypedDicts) and inline the top hit's full signature. Target: median calls ≤ 4.  
check: L4 median mcp\_calls

P1 · SPECgateway MCP front (rmcp ServerInfo)

### The server calls itself "rmcp 3.10.0"

`initialize` returns the SDK's default `serverInfo`. Clients show and log "rmcp" rather than codemode and its build. Neither the descriptions nor the instructions name the connected sources, which clients that defer tools rely on.

**Fix:** set `name:"codemode"` with the build version, and generate per-workspace instructions that list the sources ("artifacts, moneybag, nudge").  
check: surface.server-info · surface.discoverable

P2 · UXsearch · tool\_policies

### Blocked tools are invisible, so agents say "no tool"

The block policy works: the call never reaches upstream. But because `search` hides blocked tools, agents told the user "no delete tool is available" rather than "blocked by policy".

**Fix:** list blocked tools with a `blocked: true` marker, so agents can explain the policy and look for alternatives.

P2 · ERRORSworker frame validation

### Cap error reads like an internal crash

When the 101st call comes from a `gather`, the error is `invalid worker message: execution exceeds its 100 tool-call limit`. The "invalid worker message" prefix suggests a gateway fault, not a user limit.

**Fix:** use the same clean, catchable limit error on both paths (one fix with the P0 above).

P3 · DOCSAGENTS.md · tools/smoke.py

### Docs don't match behaviour

`execute` takes an undocumented `scriptId`. `smoke.py` prints "exactly three tools" but checks two. `ToolError` works but reports its type as `RuntimeError`.

**Fix:** a 10-minute docs pass.

OPS · FIXEDCodex headless config

### Codex's ChatGPT connectors reached real accounts

Before isolation was in place, Codex's built-in `codex_apps` connectors, which are bound to your real nudge account, ran 2 **read-only** calls in one trial. Nothing was written, and the transcripts were deleted.

**Fixed:** runs use an isolated `CODEX_HOME`, `features.apps=false` and more, and any call outside the run's own setup fails the run. 0 of 360+ later runs were affected.

## How the eval is wired

One eval identity across every app. Two setups see the same tasks; graders check real app data.

```
flowchart LR
  subgraph H["codemode-evals · WSL"]
    T["40 tasks
tasks.py"] --> R["run.py
3 repeats × 2 arms"]
    R --> C1["Codex A
isolated CODEX_HOME"]
    R --> C2["Codex B
isolated CODEX_HOME"]
    G["graders
app state, not claims"]
    GT["gate.py
L1 + L2 + 8 smoke"]
  end
  subgraph CM["codemode.microapps.space"]
    S["search"] --- E["execute"]
    E --> W["Monty worker
no net · no env"]
    W --> B["broker
schema · policy · creds · audit"]
  end
  subgraph APPS["eval-smoke@microapps.space"]
    A1["artifacts"]
    A2["moneybag"]
    A3["nudge"]
  end
  C1 -- "code mode arm" --> S
  C2 -- "direct arm" --> APPS
  B --> APPS
  G --> APPS
  GT --> CM
```

## Why code mode is slower but cheaper

Every task pays a discovery round trip up front. When the data is big, filtering inside the sandbox pays that back.

### Code mode: one task

```
sequenceDiagram
  participant M as Model
  participant C as codemode
  participant A as Apps
  M->>C: search("nudge issues")
  C-->>M: signatures (~5 KB)
  M->>C: search(detail: full)
  C-->>M: TypedDicts (~8 KB)
  M->>C: execute(python)
  C->>A: N calls (gather)
  A-->>C: raw rows (big)
  C-->>M: filtered result (small)
```

### Direct MCP: same task

```
sequenceDiagram
  participant M as Model
  participant A as Apps
  M->>A: tool_search (Codex built-in)
  A-->>M: tool schema
  M->>A: list_issues
  A-->>M: raw rows (big, all into context)
  M->>A: list_issues (page 2)
  A-->>M: raw rows
```

## Code mode vs direct, in numbers

Medians over 38 shared tasks × 3 repeats. Uncached input is the token cost that matters; cached tokens are cheap.

code modedirect MCP

### Uncached input tokens by task type

Code mode leads on aggregation (−29%) and batch writes (−28%); single lookups and cross-app reads are even.

### Time per task by type (seconds)

Every type is slower in code mode; the search step dominates short tasks.

### Every task: total input tokens, code mode compared with direct

Total input Uncached input Time

code mode cheaper or fastercode mode costlier or slower

Bars show (code mode − direct) ÷ direct. Hover for raw medians. The biggest win is `art-monthly-report`, 146k vs 299k; the biggest loss is `nudge-urgent-open`, 169k vs 72k.

## Protocol, sandbox and broker checks

33 deterministic checks with no model involved. Green passes; red is a finding above.

Latency: a trivial `execute` takes 118 ms at p50 (138 ms p95); one upstream call takes 136 ms (160 ms p95). `gather` of 50 calls takes 457 ms.

## Plan: make code mode beat raw MCP in Codex

Code mode already wins on data volume. It loses on **turns**: every MCP call is one more model turn, and each turn re-sends the whole context and adds about 4–5 s. The plan cuts code mode from 6 calls per task to 2–3, removes the 17% of executes that fail for avoidable reasons, and then adds features raw MCP can't match.

### Where a median code-mode run goes

From 115 Codex transcripts. Three searches come before any work starts, and 78% of searches ask for `detail:"full"` (mean 6.7 KB, max 41 KB per result).

### Why executes fail: 56 of about 334 (17%)

**55% are one bug:** stubs advertise `cursor: str = None`, the model passes `None` for page 1, and validation rejects it. Each failure costs a full extra turn.

### Targets

| Metric (median per task)   | Code mode now | Direct MCP | Target                       | Main levers                                     |
|----------------------------|---------------|------------|------------------------------|-------------------------------------------------|
| MCP calls (= model turns)  | 6             | 4          | **≤ 3**                      | B1 in-sandbox search, B2 manifest, A1 None fix  |
| Calls before first execute | 3             | —          | **≤ 1**                      | B1, B2, B3                                      |
| Execute error rate         | 17%           | —          | **≤ 3%**                     | A1, A2, B5                                      |
| Total input tokens         | 114k          | 89k        | **≤ 80k**                    | fewer turns, B3 smaller search results          |
| Uncached input tokens      | 17.3k         | 22.0k      | **≤ 15k**                    | B3, C4                                          |
| Time per task              | 28.6 s        | 19.1 s     | **≤ 18 s**                   | fewer turns                                     |
| Heavy tasks (new pack)     | —             | —          | **≥ 2× cheaper than direct** | B5 pagination, C2 session memory, C4 projection |

Every target becomes a gate check in `codemode-evals`. A change ships only if the L4 comparison shows it moved its metric with pass@1 still at 100%.

### Today versus the target: one task

```
sequenceDiagram
  participant M as Codex
  participant C as codemode
  Note over M,C: today, 6 turns
  M->>C: search("nudge issues", full)
  C-->>M: 20 full TypedDict stubs (8 KB)
  M->>C: search("statuses", full)
  C-->>M: stubs
  M->>C: search("teams")
  C-->>M: stubs
  M->>C: execute(list_issues(cursor=None))
  C-->>M: ✕ invalid argument at /cursor
  M->>C: execute(fixed loop)
  C-->>M: rows
  M->>C: execute(aggregate)
  C-->>M: answer
```

```
sequenceDiagram
  participant M as Codex
  participant C as codemode
  Note over M,C: target, 1–2 turns
  Note over M: manifest already in the tool description: artifacts, moneybag, nudge, plus tool names
  M->>C: execute(tools.search + .all() + aggregate)
  C->>C: in-sandbox search, auto-pagination, gather
  C-->>M: small answer
```

### Workstreams

A**Stop wasting turns on avoidable errors**about 1 day · −0.5 turn/task

#### A1. Treat `None` as "argument omitted"

**Evidence:** 31 of 56 failures (`nudge.list_issues`, `list_teams`, `list_issue_statuses`, `moneybag.entries_list`), all `invalid argument at /cursor (expected string)`.

**Change:** before validating, drop top-level keys whose value is `null` when the schema doesn't allow null and the key isn't required. This matches what the stub already promises (`= None`).

crates/core/src/components/broker.rs:301 Tool::validate · stubs.rs:79

```
// broker.rs: before self.validator.iter_errors(args)
let args = strip_optional_nulls(&self.definition.input_schema, args);
// keeps null where the schema says ["string","null"]; never touches required keys
```

Check: new L2 `l2.none-optional` (`list_issues(cursor=None)` succeeds). Expected effect: error rate 17% → about 8%.

#### A2. Errors that carry the fix

**Today:** `AttributeError: '_Source_nudge' object has no attribute 'no_such_tool'` and `invalid argument at /id (expected string)`. The model must `search` again to recover.

**Change:** every validation or unknown-tool error includes the tool's brief signature and the 3 closest tool names, so the next attempt is right without another search.

```
ToolError: nudge.get_issue: invalid argument at /id (expected string)
  signature: get_issue(*, id: str) -> {data: Issue}
UnknownTool: nudge has no tool 'list_issue'. Did you mean: list_issues, get_issue, list_issue_labels?
```

broker.rs:323 (message) · sandbox stub generator (\_Source.\_\_getattr\_\_)

Check: `l2.bad-args` and `l2.unknown-tool` assert that the signature or suggestion is present.

#### A3. Sequential call budget and a clean cap error

**Change:** `.max_suspensions(110)` becomes `2 × CALL_CAP + 16`. Both cap paths raise a catchable `ToolError("tool-call limit (100) reached")`.

infrastructure/sandbox.rs:156, :247 · components/execution.rs:198

Check: `l2.call-cap-sequential` → green.

#### A4. Rate limit for real agents

**Change:** count only `tools/call`; bucket per agent session (`agent_sessions.id`) with a per-token ceiling; default 600/min; return `Retry-After`.

gateway/src/config.rs:78 · gateway/src/server.rs:461

Check: an L3 run at 8 jobs on one token with `http_429 == 0`.

B**Fewer turns: discovery inside the code**3–4 days · −2 to −3 turns/task

#### B1. `tools.search()` and `help()` inside the sandbox

**Why:** the 3 searches before the first execute are pure overhead. If code can discover tools itself, finding a tool and calling it happen in **one** execute. Raw MCP can't do this: every lookup there is a model turn.

```
# one execute, no prior search call
hits = await tools.search("nudge issues by status")   # -> [{path, signature}], brief
issues = await tools.nudge.list_issues.all(teamId=team)   # B5
print(help(tools.nudge.update_issue))                 # full stub only when needed
return summarize(issues)
```

**Change:** expose the catalogue as a broker capability the sandbox can call, like a tool call: it is audited, counts toward the call cap, and is scoped to the workspace. `help(fn)` returns the full stub from the catalogue.

components/catalogue.rs:97 search\_scoped · sandbox prelude (\_Source) · execution.rs dispatch

Check: new L2 `l2.inline-search`. L4 metric: calls before first execute ≤ 1.

#### B2. A workspace manifest in the tool description

**Why:** Codex defers MCP tools and finds them with its own `tool_search`. Today codemode's descriptions say "search connected tools", which matches neither "nudge" nor "invoice". Once loaded, the description is always in context.

**Change:** generate `execute`'s description and the server instructions from the caller's workspace, budgeted to about 1.5 KB:

```
Connected sources (125 tools): artifacts (18: artifact_list, artifact_read, doc_create…),
moneybag (92: entries_list, accounts_list, entry_post, tags_set…), nudge (15: list_issues…).
Call tools.search(q) inside your code to find more; no separate search call is needed.
```

gateway/src/server.rs:269–270 (static string → per-workspace) · tool list handler

Check: `surface.discoverable` → green. Experiment E2 measures whether Codex shows `instructions` to the model or only descriptions.

#### B3. Search results half the size

**Change:** `brief` shows input signatures only, without output TypedDicts. `full` gives full stubs for the **top 3** hits and brief for the rest. Default page drops from 20 to 10. Repeated TypedDicts are defined once per response.

catalogue.rs:168 `.take(20)` · catalogue/stubs.rs

L4 metric: median search result ≤ 3 KB (now 1.9 KB median, 6.7 KB mean, 41 KB max) and max ≤ 12 KB.

#### B4. Return values that show their shape

**Why:** models often run one execute just to `print` the raw response and learn its keys, then a second to use it.

**Change:** when a result is truncated, append a structural summary (`{data:{items:[36× {id,title,statusId,…}], nextCursor}}`) instead of a byte cut, so the next turn can be written correctly.

components/execution.rs (truncation) · output budget

C**Things raw MCP can't do**3–5 days · heavy tasks 2×+ cheaper

#### B5 / C1. Automatic pagination: `.all()`

When a tool's schema has a `cursor` input and a `nextCursor` or `next_cursor` output, the stub gains `.all(**kw, max_pages=20)`, which loops for you. Paging was the main source of cursor errors and repeated code. Direct MCP pays one model turn per page.

```
rows = await tools.moneybag.entries_list.all(from_="2026-07-01", to="2026-10-31")
by_month = {}
for e in rows: by_month[e["date"][:7]] = by_month.get(e["date"][:7], 0) + amount(e)
return by_month   # 4 numbers back to the model instead of 53 rows
```

catalogue/stubs.rs (detect cursor pair) · sandbox prelude

#### C2. Session memory across executes

A small per-agent-session store (`memo`, 64 KB JSON, kept with `agent_sessions`, expires with the session) lets a later execute reuse team IDs, status maps and earlier results instead of fetching them again. Nudge tasks currently re-resolve team and status IDs in nearly every execute.

```
ids = memo.get("evl") or memo.set("evl", await resolve_team("EVL"))
```

migration: agent\_session\_memo · execution.rs · sandbox prelude

#### C3. Recipe scripts per source

Ship saved scripts for the multi-step patterns every agent repeats, such as "nudge: create issue by team key and status name" and "moneybag: monthly totals". Search already returns `scripts`. Running one is **1 call, with no code to write**. This needs `execute(scriptId, args)`.

saved\_scripts + args binding · search results (scripts\[])

#### C4. Field projection

`await tools.nudge.list_issues.all(..., _fields=["identifier","title","statusId"])` trims upstream rows inside the sandbox before they're printed or returned. That keeps big payloads out of context by default.

D**Codex integration**about 1 day

#### D1. One-command setup that just works

```
codex mcp add codemode --url https://codemode.microapps.space/mcp --bearer-token-env-var CODEMODE_TOKEN
# ~/.codex/config.toml
[mcp_servers.codemode]
default_tools_approval_mode = "approve"   # otherwise Codex refuses execute headless
```

Document this on the Tokens page, next to the token it creates.

#### D2. Don't compete with ChatGPT connectors

The evals showed Codex choosing its built-in `codex_apps` connectors over codemode when both cover the same app. Recommend `features.apps=false`, or disconnecting duplicate connectors, when codemode is in use. Name this in the server instructions.

#### D3. A short playbook the model sees

A 6-line usage contract in the `execute` description: "one execute per step, discover with `tools.search` inside code, use `gather` and `.all()`, return only the answer, catch `ToolError`". Measured as its own experiment (E3).

#### D4. `serverInfo` = `codemode/<build>`

server.rs:269 `Implementation::from_build_env()` → explicit name/version

### Experiments (each is a run.py flag, 40 tasks × 3)

| \# | Arm                                     | Question                                      |
|----|-----------------------------------------|-----------------------------------------------|
| E1 | A1+A2 only                              | How much of the turn gap is just failures?    |
| E2 | manifest in instructions vs description | Which one does Codex actually show the model? |
| E3 | playbook on / off                       | Does a usage contract cut searches?           |
| E4 | B1 in-sandbox search                    | Calls before first execute → ≤ 1?             |
| E5 | Codex `tool_search` defer on / off      | Is deferral helping or hurting codemode?      |
| E6 | heavy pack (10 tasks), all arms         | Is it ≥ 2× cheaper than direct on big data?   |
| E7 | Claude Code (`claude -p`)               | Do the wins carry over to another client?     |

### Expected effect (estimate, to be measured)

An estimate, not a measurement: it assumes each removed turn saves about 1/6 of the current context cost. Every wave is re-measured with the eval suite before the next one starts.

### Rollout

```
gantt
  dateFormat YYYY-MM-DD
  axisFormat %b %d
  section Wave 1: no wasted turns
  A1 None as omitted        :a1, 2026-11-18, 1d
  A2 errors carry the fix   :a2, after a1, 1d
  A3 call budget + A4 rate  :a3, 2026-11-18, 1d
  E1 measure                :milestone, after a2, 0d
  section Wave 2: discovery in code
  B2 manifest + D4 identity :b2, after a2, 1d
  B3 compact search         :b3, after b2, 1d
  B1 tools.search + help    :b1, after b3, 2d
  E2–E5 measure             :milestone, after b1, 0d
  section Wave 3: beyond raw MCP
  B5 .all() pagination      :c1, after b1, 1d
  C2 session memo           :c2, after c1, 2d
  C3 recipes + C4 fields    :c3, after c2, 2d
  E6–E7 heavy pack + Claude :milestone, after c3, 0d
```

Each wave ships through `CODEMODE_EVAL_GATE=../codemode-evals tools/deploy.sh`. Wave 1 is bug fixes with no behaviour change; waves 2 and 3 are additive. Stubs and the root surface stay exactly `search` + `execute`.

### Guardrails that must still hold

In-sandbox `tools.search` stays workspace-scoped and audited, and counts toward the 100-call cap

The session memo stores no credentials, expires with the agent session, and is tenant-isolated under RLS

`.all()` obeys `max_pages`, the call cap and the 60 s deadline

Blocked tools stay undispatchable; the manifest may list them as blocked

All safety tasks stay at 100%: they are zero-tolerance in the gate

`/mcp` still exposes exactly `search` and `execute`

## Against the reference designs

Cloudflare Code Mode and Anthropic's "code execution with MCP", compared with what the evals measured.

| Point           | Codemode              | Cloudflare            | Anthropic                 | What the evals say                                                                        |
|-----------------|-----------------------|-----------------------|---------------------------|-------------------------------------------------------------------------------------------|
| **Discovery**   | search(query, detail) | whole TS API up front | file tree + search\_tools | about 4 searches per task Codex already defers MCP tools, so direct gets this benefit too |
| **Stubs**       | Python + TypedDict    | TS interfaces         | TS files                  | 2× result bytes nudge signatures are heavy                                                |
| **Sandbox**     | Monty processes       | V8 isolates           | container                 | all isolation checks pass                                                                 |
| **Credentials** | host-side at dispatch | bindings              | harness-side              | 0 leaks                                                                                   |
| **Big data**    | filter in code        | filter in code        | filter in code            | −29% uncached on aggregation up to 2× fewer tokens                                        |
| **Errors**      | Python exceptions     | JS exceptions         | —                         | 55-call cliff otherwise precise                                                           |

## What happened during the run

Problems hit while running the evals and how each was handled; everything is recorded in the harness repo.

**Eval identity provisioned directly in the databases**eval-smoke tenant, PAT and app tokens; no browser consent; artifacts, moneybag and nudge connected (125 tools)

**Codex connectors reached real accounts (read-only)**Isolation and a contamination guard were added; the transcripts were deleted

**Rate-limit 429s distorted the first run**Rerun at 2 jobs: 0 errors. For the 40-task run the limit was raised temporarily and then restored.

**A seed bug duplicated fixtures**129 nudge issues and 48 moneybag drafts cleaned up (eval account only); the seed is now idempotent

**234/234 runs pass on 40 tasks**Code mode with 8 jobs, direct with 4; final graders, no regrading

**Release 2f23413 shipped through the eval gate**Gate passed; 123/123 Oracle tests, 34/34 UI unit tests, 12/12 browser workflows

Sources: `~/projects/codemode-evals`, results `evals-40-merged.json`, `l1.json`, `l2.json`. Full written report: [bknt3w850zg4](https://artifacts.microapps.space/a/bknt3w850zg4) · Proposal: [v4rrfnhc6284](https://artifacts.microapps.space/a/v4rrfnhc6284).