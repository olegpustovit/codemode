# Codemode MCP Gateway

One control plane where every tool is connected once and exposed to agents as a small MCP server on `*.microapps.space`. Agents search a catalogue and write Python that calls the tools, instead of loading hundreds of tool definitions.

Implementation plan: https://artifacts.microapps.space/a/saqdh2xm7jcd (revision 5). Read it with `art read saqdh2xm7jcd` before starting a phase: it has the task lists, data model, component behaviour and acceptance checks. This file is the summary.

## Status

Phase 0 is implemented and measured on `oracle-vm`; see `docs/spike.md` and `docs/status.md`. All nine technical criteria pass; the owner must read the report before Phase 1 starts. No application service is deployed yet. Do not build ahead of the current phase.

## Stack

- Rust, Cargo workspace, Tokio, axum.
- `rmcp` (official MCP SDK), Streamable HTTP in stateless mode.
- sqlx on Postgres, hand-written SQL, embedded forward-only migrations.
- Monty (`monty`, `monty-pool`) as the sandbox, in worker processes.
- Control plane: Vite front end embedded in the binary (Phase 2).
- Deploy: one service binary plus the Monty worker binary, under smdctl, behind Caddy.

Follow the `build-rust-stack` skill for workspace layout, components, errors, configuration, tests and build budgets, and `smdctl-deploy` for deployment. Run tests through the `cargo t` wrapper that skill sets up (hard deadlines: 2 s unit, 5 s integration).

## MCP surface

Exactly three tools. Do not add more without asking.

| Tool | Input | Behaviour |
|---|---|---|
| `search` | `query`, `detail`, `cursor` | Full-text search over the tools this caller may reach. Returns path, one-line description, input type. `detail: "full"` adds output type and docs. |
| `execute` | `code` | Runs Python in the sandbox. The only capability is the `tools` object. Returns the return value plus captured output, truncated at a fixed budget. |
| `resume` | `executionId` | Continues an execution that paused for approval. |

## Components

- **MCP front**: `rmcp` service on axum. No MCP sessions; a paused execution is addressed by its own id. One endpoint must serve both a 2026-07-28 client and a 2025-11-25 client.
- **Sources**: remote MCP over Streamable HTTP, local stdio MCP servers, OpenAPI specs (Phase 3). GraphQL waits.
- **Catalogue**: tool JSON Schemas converted to Python stub signatures, indexed in Postgres full-text search. Schemas the generator cannot express fall back to `Any`.
- **Sandbox**: a Monty worker from a warm pool. On a tool call the worker pauses and hands the gateway the pending calls; the gateway runs them concurrently on Tokio and resumes the worker. Keep it behind a `Sandbox` interface so the engine can be replaced.
- **Tool broker**: validates arguments against the tool's JSON Schema, applies policy, attaches credentials, calls upstream, writes the audit row.
- **Control plane**: sources, linked accounts, policies, execution log, approvals.

## Rules that must hold

- **Never run Monty in the gateway process.** Untrusted code runs only in `monty-pool` workers started with an empty environment. A crash or interpreter bug must end one worker, not the gateway.
- **The sandbox has no network and no secrets.** Its single exit is the broker. Credentials are attached on the host side at dispatch.
- **Never pass the agent's token to an upstream.** Every upstream gets its own linked credential, stored in Postgres and encrypted with a key from the service environment.
- **Tokens are bound to one hostname's audience.** A token for `work` is rejected at `home`. Each hostname publishes its own protected-resource metadata.
- **The gateway validates every frame from a worker** and signs any snapshot it stores; Monty snapshots are not authenticated.
- **Approvals happen in the browser.** The agent can collect the answer through `resume` but can never supply it. Never replay a paused execution: replaying repeats side effects.
- **Every execution is audited**: the code, each tool call with redacted arguments, timing and outcome.
- **Tool failures surface as Python exceptions** the code can catch, with precise messages. Nobody type-checks model code, so error quality is the feedback loop.
- **Pin Monty and `rmcp` to exact versions.** Both change APIs between minors; `rmcp` has a history of security advisories, so check them when upgrading.

## Starting limits

| Limit | Value |
|---|---|
| Execution time | 60 s (many MCP clients time out a tool call at about 60 s) |
| Memory per worker | 64 MB |
| Tool calls per execution | 100 |
| Source size | 64 KB |

## Performance

The gateway is network-bound. Write plain owned code everywhere except three paths, each of which gets a stated allocation budget and a test that fails when it is exceeded:

1. Catalogue search.
2. Passing tool results through without parsing them into a tree.
3. Framing between gateway and workers.

## Policy defaults

Per tool: allow, require approval, or block. GET requests and tools marked read-only are allowed; tools marked destructive ask first.

## Domains

- `codemode.microapps.space`: control plane, authorization server, and an "everything" endpoint at `/mcp`.
- `<slug>.microapps.space/mcp`: one endpoint, a named bundle of sources plus a policy.
- Caddy keeps its explicit site blocks and gains a wildcard block that falls through to the gateway. The gateway reserves existing names so a slug cannot shadow another app.
- Agent auth: OAuth 2.1 with PKCE, client ID metadata documents first, dynamic client registration kept for older clients, personal access tokens for headless agents.

## Plan

| Phase | Scope | Done when |
|---|---|---|
| 0 | Spike: worker pool against the pass criteria, dual-era MCP serving with `rmcp`, twenty-task Python trial | Every criterion has a measured result |
| 1 | Sources from a config file (remote and stdio MCP), stub generation, catalogue, `search`, `execute` with limits and truncation, token auth, audit table, single hostname. No UI | Used daily in place of direct MCP connections |
| 2 | Web UI, authorization server and per-host metadata, upstream OAuth linking, encrypted store, subdomain endpoints | claude.ai connects to a subdomain through OAuth |
| 3 | OpenAPI sources, per-tool policy, pause and `resume`, approval page | A destructive call waits for a click |
| 4 | OS-level worker isolation, rate limits, per-endpoint scopes, GraphQL, saved scripts | Only when another person gets access |

### Phase 0 pass criteria

- `asyncio.gather` over 50 tool calls runs them concurrently on the gateway side.
- `while True: pass` is stopped at the deadline, every time.
- A run that allocates past its cap ends its own worker; the gateway and other runs are unaffected.
- Killing a worker mid-run produces a clean error to the agent.
- 10,000 sequential runs leave gateway memory flat.
- A warm run starts in under 5 ms on this arm64 box.
- A paused run restores from its snapshot after a gateway restart, and a tampered snapshot is rejected.
- One endpoint serves both a 2026-07-28 client and a 2025-11-25 client.

If the pool fails these, the fallback is `deno_core` with TypeScript in a worker process. Report the measured results; do not pick the fallback without asking.

## Open questions: ask, do not assume

- Who uses it: only the owner, or other people too. This sets the threat model.
- Flat subdomains (`work.microapps.space`) or a sub-zone (`work.mcp.microapps.space`).
- Control-plane sign-in: reuse the Artifacts identity or stand alone.
- Paused executions are stored as signed snapshots in Postgres with a short expiry and continue from the paused call. Phase 0 must prove snapshot restore works before Phase 3 depends on it.
- How `monty-pool` enforces the per-worker memory cap is unverified; the spike measures it.
