# Codemode MCP Gateway

One control plane where each user connects their own tools once and exposed to agents as a small MCP server at `codemode.microapps.space/mcp`. Agents search a catalogue and write Python that calls the tools, instead of loading hundreds of tool definitions.

Implementation plan: https://artifacts.microapps.space/a/saqdh2xm7jcd (revision 5). Read it with `art read saqdh2xm7jcd` before starting a phase: it has the task lists, data model, component behaviour and acceptance checks. This file is the summary.

## Status

Phases 0–4 and the workspace/administration extension are implemented and deployed on `oracle-vm` under smdctl, loopback `127.0.0.1:2093`, at `https://codemode.microapps.space`. Current schema: **16**. Release SHA-256: `20e07441525c8d2125baa1b3aaaf41d6d4d50bd50d298e98df011b3f8e85390b`. UI fingerprint: `f22d8e0dde58`. The React frontend is embedded in this Rust binary.

Current qualification passes **90 native**, **90 hardened Oracle** checks and **seven optimized-binary browser workflows**, including tenant isolation, mandatory OAuth workspace selection, administration-resource separation, both MCP eras, provider presets, policies, approvals and saved scripts. The fresh visual reviewer returned **ship** with no material fixes. See `docs/status.md` and `docs/measurements/workspaces-qualification.json`. Incremental build latency currently exceeds the 10 s core / 5 s gateway targets; keep that limitation visible rather than claiming those budgets pass.

The owner authorized continuing through manual gates. Live-provider consent and external-client acceptance remain unperformed; do not create production fixtures or grant provider consent on the user's behalf. Custom domains and wildcard Caddy acceptance are superseded by the owner's single-host workspace decision. Existing source connections, IDs, credentials, policies, scripts and audit history survive the workspace migration. Deployment preserves the registered user and does not create accounts.

## User accounts and tenancy

- Control-plane access uses email and password. With no users, show registration; the first registered account has `admin: true`. There is no super-admin role or tenant-isolation bypass.
- Registration permits exactly `ursus95@gmail.com` and email addresses at `olegpustovit.com` (case-insensitive).
- Each user owns a private tenant: connected MCPs, linked upstream OAuth accounts and credentials, catalogue, workspaces, agent tokens, policies, scripts, approvals and execution history.
- Agent OAuth consent grants access only to the signed-in user's tenant. A shared hostname never implies shared tools or credentials.
- The shared administrator password is retired. Existing sessions and agent tokens are revoked when tenancy is introduced.

Tenancy rollout baseline (superseded by the current Status above): account registration and private tenancy were deployed on Oracle, schema **14**, binary SHA-256 `1ebf8358c1c028d32cf9c9ec2c8bab385efb53aea7c2bbda8d6eef60d2989fb0`. Qualification passes **89 native**, **89 hardened Oracle** checks and **six optimized-binary browser workflows**. Public HTTPS registration and mobile checks pass; deployment created no users. Existing sessions, agent tokens and OAuth grants were revoked. Email verification and password recovery are not implemented. See the current account section in `docs/status.md`.

## Stack

- Rust, Cargo workspace, Tokio, axum.
- `rmcp` (official MCP SDK), Streamable HTTP in stateless mode.
- sqlx on Postgres, hand-written SQL, embedded forward-only migrations.
- Monty (`monty`, `monty-pool`) as the sandbox, in worker processes.
- Control plane: build-react-stack frontend embedded in the Rust server binary (Phase 2). Follow the `build-react-stack` skill for the frontend, adapting its Go-specific integration to Rust; the backend remains on `build-rust-stack`.
- Deploy: one service binary plus the Monty worker binary, under smdctl, behind Caddy.

Follow the `build-rust-stack` skill for workspace layout, components, errors, configuration, tests and build budgets, and `smdctl-deploy` for deployment. Run tests through the `cargo t` wrapper that skill sets up (hard deadlines: 2 s unit, 5 s integration).

## MCP surface

The `/mcp` execution surface has exactly three tools. Do not add more without asking.

The owner also authorized a separate `/mcp-admin` management surface: workspace CRUD, source CRUD/sync, account metadata and browser account-link URLs. It requires its own OAuth resource and `admin` scope (or a management PAT), never an ordinary `/mcp` token. Management always stays within the signed-in user’s tenant; the first-user admin attribute grants no cross-tenant access. Never return credentials or grant provider consent through a tool. Audit every management invocation.

| Tool | Input | Behaviour |
|---|---|---|
| `search` | `query`, `detail`, `cursor` | Full-text search over the tools this caller may reach. Returns path, one-line description, input type. `detail: "full"` adds output type and docs. |
| `execute` | `code` | Runs Python in the sandbox. The only capability is the `tools` object. Returns the return value plus captured output, truncated at a fixed budget. |
| `resume` | `executionId` | Continues an execution that paused for approval. |

## Components

- **MCP front**: `rmcp` service on axum. No MCP sessions; a paused execution is addressed by its own id. One endpoint must serve both a 2026-07-28 client and a 2025-11-25 client.
- **Sources**: remote MCP over Streamable HTTP, local stdio MCP servers, OpenAPI specs (Phase 3), and GraphQL query/mutation sources (Phase 4).
- **Catalogue**: tool JSON Schemas converted to Python stub signatures, indexed in Postgres full-text search. Schemas the generator cannot express fall back to `Any`.
- **Sandbox**: a Monty worker from a warm pool. On a tool call the worker pauses and hands the gateway the pending calls; the gateway runs them concurrently on Tokio and resumes the worker. Keep it behind a `Sandbox` interface so the engine can be replaced.
- **Tool broker**: validates arguments against the tool's JSON Schema, applies policy, attaches credentials, calls upstream, writes the audit row.
- **Control plane**: sources, linked accounts, policies, execution log, approvals.

## Rules that must hold

- **Never run Monty in the gateway process.** Untrusted code runs only in `monty-pool` workers started with an empty environment. A crash or interpreter bug must end one worker, not the gateway.
- **The sandbox has no network and no secrets.** Its single exit is the broker. Credentials are attached on the host side at dispatch.
- **Never pass the agent's token to an upstream.** Every upstream gets its own linked credential, stored in Postgres and encrypted with a key from the service environment.
- **Tokens are bound to a workspace and resource.** OAuth consent selects a workspace for `/mcp`; its source membership and policies govern search and dispatch. `/mcp-admin` requires separate management authorization. The canonical host publishes protected-resource metadata for each path.
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

## Workspaces and connection URLs

- `codemode.microapps.space` is the only supported host. No wildcard routing, custom domains or subdomain configuration.
- `/mcp` is the shared execution URL. During OAuth consent the user explicitly chooses a named workspace containing selected sources. Access and refresh tokens retain that workspace selection.
- `/mcp-admin` is the separate management URL for the user's own tenant. Its consent explicitly grants administration access, not workspace execution access.
- Each tenant has a protected default “All sources” workspace. Other workspaces have a human name, selected sources, tool scopes, policies and saved scripts.
- The Sources page offers popular MCP presets. Adding a preset creates a source; linking and provider consent happen in the browser.
- Keep existing IDs, sources, credentials, memberships, scripts, policies and audit history during migration. Legacy custom-host grants are revoked rather than silently reassigned to a different OAuth resource.
- Agent auth: OAuth 2.1 with PKCE, client ID metadata documents first, dynamic client registration for older clients, personal tokens for headless agents.

## Plan

| Phase | Scope | Done when |
|---|---|---|
| 0 | Spike: worker pool against the pass criteria, dual-era MCP serving with `rmcp`, twenty-task Python trial | Every criterion has a measured result |
| 1 | Sources from a config file (remote and stdio MCP), stub generation, catalogue, `search`, `execute` with limits and truncation, token auth, audit table, single hostname. No UI | Used daily in place of direct MCP connections |
| 2 | Web UI, authorization server and per-resource metadata, upstream OAuth linking, encrypted store, workspace selection | a client selects a workspace through OAuth at /mcp |
| 3 | OpenAPI sources, per-tool policy, pause and `resume`, approval page | A destructive call waits for a click |
| 4 | OS-level worker isolation, rate limits, per-workspace scopes, GraphQL, saved scripts, MCP tasks | Only when another person gets access |

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

## Owner decisions and qualified constraints

- Other people will use the gateway; Phase 4 is in scope before shared deployment.
- `codemode.microapps.space` is configured and ready. Use only this host; the owner explicitly rejected custom domains and wildcard subdomains.
- Control-plane sign-in is standalone.
- Paused executions are stored as signed snapshots in Postgres with a short expiry and continue from the paused call. Phase 0 proved snapshot restore and tamper rejection; Phase 3 must preserve these guarantees.
- Phase 0 measured the per-worker memory cap and worker-failure containment; keep the regression checks when changing the pool.
