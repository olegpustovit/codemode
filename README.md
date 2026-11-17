# Codemode MCP Gateway

Implementation of [the revision 5 plan](https://artifacts.microapps.space/a/saqdh2xm7jcd), one phase at a time.

Phase 0 qualified the Monty subprocess pool, signed cross-process snapshots, stateless dual-era MCP and a 24-task trial against real Nudge, Researcher and Artifacts binaries with disposable test databases. Its throwaway crate has been retired; worker regressions now exercise the production adapter in `crates/core/tests/it/pool.rs`.

See [measured results](docs/spike.md) and [current status](docs/status.md). Phases 0–4 plus workspaces, provider presets and administration MCP are deployed on `oracle-vm` at `https://codemode.microapps.space`, loopback **2093**, schema **16**. The React frontend is embedded in the Rust binary. Qualification passes **90 native**, **90 hardened Oracle** checks and **seven optimized-binary browser workflows**. Live-provider consent and external-client acceptance remain owner/provider checks. Incremental rebuilds currently exceed the recorded 10 s core / 5 s gateway targets; see the qualification record.

## Development

Requires Rust 1.100.0, uv and `mold`/clang on aarch64 Linux.

```sh
uv venv .tools/monty
uv pip install --python .tools/monty/bin/python pydantic-monty-runtime==1.5.0
export DATABASE_URL="postgresql://$USER@localhost/postgres?host=/var/run/postgresql"
export SQLX_OFFLINE=true
cargo t --workspace
T_SLOW=1 cargo t --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
bash tools/check-dependencies.sh
```

The database test user needs permission to create disposable test databases. `DATABASE_URL` is for the test runner; the service uses `CODEMODE_DATABASE_URL`.

## Frontend

The control plane lives in `ui/`: Vite, React with React Compiler, native TypeScript 7, TanStack Router/Query, Tailwind and shadcn Base Nova. Bun owns the lockfile and commands. The browser uses relative API paths and reads public configuration from `/api/config`.

```sh
cd ui
bun install --frozen-lockfile
bun run lint
bun run typecheck
bun run test
bun run stage
```

Staging builds and verifies the UI, then copies it into `crates/gateway/ui-dist` for Rust `include_bytes!` embedding. Input and output content hashes reject missing, stale or modified builds. Unchanged staging preserves file timestamps. Backend-only builds never invoke Bun; they identify the embedded build and warn if it is stale. `bun run --cwd ui stage --check` verifies without rebuilding. `tools/artifact.sh` produces a native release and runs the embedded HTTP and browser regression checks against that binary.

`bun run --cwd ui dev` serves port 5173 and proxies `/api` to the local gateway on port 2093. The development proxy supplies the configured control-plane Host and HTTPS Origin; production browser writes retain the strict same-origin boundary.

`bun run --cwd ui test:browser` checks a previously built `target/debug/codemode`; set `CODEMODE_GATEWAY_BIN` to check another native binary. It needs PostgreSQL database-creation privileges, the Monty worker, OpenSSL and Chromium (`PLAYWRIGHT_CHROMIUM_EXECUTABLE` overrides its path). The test creates and removes a disposable database, uses a local MCP fixture and a local TLS proxy, and maps browser hostnames only inside its Chromium process. It does not change Caddy, DNS or production data. Screenshots and logs go under ignored `target/ui-browser/`.

The Rust server serves the UI only on the main hostname. Deep links return `index.html` with `no-cache`; hashed assets have immutable caching, public icons and the manifest have short caching. Missing assets return 404, unknown API routes return JSON 404, and reserved MCP/OAuth/health routes never fall through to HTML. `python3 tools/smoke-ui.py ORIGIN HOSTNAME` checks this contract against a running binary.

## Gateway

The gateway applies embedded migrations at startup. Before the first Phase 2 startup, copy `sources.example.toml` to `sources.toml` and supply each referenced credential in the service environment, or use `sources = []` for an empty catalogue. The file is imported once into PostgreSQL; referenced credentials are encrypted, and subsequent file edits are ignored. Use the source API for later changes. An encryption key is required when importing credentials.

| Variable | Default / requirement |
|---|---|
| `CODEMODE_DATABASE_URL` | Required PostgreSQL connection URL |
| `CODEMODE_HOSTNAME` | `codemode.microapps.space` |
| `CODEMODE_ID_ACCESS_ENABLED` | `false`; central eligibility requires confidential Microapps ID mode; see [access-groups PR status](docs/access-groups.md) |
| `CODEMODE_BIND` | `127.0.0.1:2093`; loopback required |
| `CODEMODE_SOURCES_FILE` | `sources.toml`; one-time initial import |
| `CODEMODE_MONTY_BIN` | `./monty`; use the installed runtime binary |
| `CODEMODE_ENCRYPTION_KEY` | 64 hexadecimal characters; required to store linked credentials |
| `CODEMODE_ENCRYPTION_KEY_ID` | `primary`; identifier stored with every encrypted credential |
| `CODEMODE_LOG_JSON` | `false` |
| `CODEMODE_LOG` | `info`; tracing filter |
| `CODEMODE_RESULT_BYTES` | `32768`; range 256–1048576 |
| `CODEMODE_OUTPUT_BYTES` | `32768`; range 256–1048576 |
| `CODEMODE_CONCURRENT_EXECUTIONS` | `8`; range 1–64, excess requests fail busy |
| `CODEMODE_CONCURRENT_CALLS` | `16`; range 1–100 per execution |
| `CODEMODE_UPSTREAM_TIMEOUT_SECS` | `30`; range 1–60, bounded by the execution deadline |
| `CODEMODE_TOKEN_REQUESTS_PER_MINUTE` | `60`; range 1–60000; authenticated MCP request budget per token |
| `CODEMODE_ENDPOINT_REQUESTS_PER_MINUTE` | `600`; range 1–600000; authenticated MCP request budget shared by an endpoint |

With the database configured, `codemode token create NAME` prints a new bearer token once. `codemode token list` lists metadata and `codemode token revoke UUID` revokes one. Tokens are bound to `CODEMODE_HOSTNAME`.

Open `https://codemode.microapps.space`. With no users, the site shows email/password registration. Registration permits `ursus95@gmail.com` and addresses at `olegpustovit.com`; email matching is case-insensitive. The first account receives `admin: true`; every account owns a separate tenant. Passwords use Argon2id (12–1024 bytes). Browser sessions expire after 12 hours, revoke on logout or password changes, and use a Secure, HttpOnly, host-only cookie. Writes to `/api` require the control plane's exact HTTPS Origin. The old shared administrator password no longer grants access. Email ownership verification and password recovery are not implemented.

MCPs, linked credentials and upstream OAuth connections, workspaces, tokens, catalogue entries, policies, scripts and history are private to the account's tenant. Both browser APIs and agent execution use immutable tenant database pools under an unprivileged PostgreSQL role with row-level security. OAuth consent selects a workspace within the signed-in user's tenant; authorization-code and refresh exchanges preserve that selection. Operator CLI tokens belong to the initial tenant; use the browser to issue other tenants' agent tokens. Local stdio commands are operator-managed through the initial sources configuration; browser users can connect HTTP MCP, OpenAPI and GraphQL sources.

Phase 2 backend work adds session, source, token, encrypted-account and execution-log APIs. Linked credentials use [RustCrypto's XChaCha20-Poly1305](https://docs.rs/chacha20poly1305/0.11.0/chacha20poly1305/) with random 24-byte nonces and source-bound authenticated data. API account responses contain metadata only. The broker loads HTTP credentials at request dispatch; stdio children receive only their linked environment. Linking, rotating or removing credentials invalidates existing connections. Changing a source's destination removes its old linked credential. Source Python namespaces remain stable across renames and naming collisions.

Sources support create/list/read/update/delete through `/api/sources` and `/api/sources/{id}`, with `/api/sources/{id}/sync` for a synchronization request. Link a bearer credential with `PUT /api/accounts/{id}` and a stdio environment with `PUT /api/accounts/{id}/environment`. Source writes cannot reference arbitrary gateway environment variables; those references are accepted only during the initial file import. The React frontend is deployed; provider consent and live token exchange remain pending.

Start upstream OAuth linking with session-authenticated `POST /api/accounts/{id}/oauth`, which returns an `authorizationUrl`. The official `rmcp` client discovers the provider, selects client metadata documents when advertised, otherwise registers dynamically, and requests S256 PKCE. Its public client document is served at `/oauth/client-metadata.json`. The provider returns the browser to `/api/accounts/oauth/callback`; this callback requires the same browser session, validates the issuer and consumes its encrypted state once. State expires after ten minutes, survives a gateway restart, and cannot replace an account after its source changes. A successful callback redirects to the source account page and requests synchronization. Provider denial preserves the existing account.

Upstream OAuth access/refresh tokens, client secrets, PKCE verifiers and metadata snapshots are encrypted with the account key. The broker checks the destination URL and refreshes credentials within 30 seconds of expiry. Concurrent calls share a refresh, persisted rotation uses a nonce comparison, and an in-flight refresh cannot overwrite a replacement account. Temporary provider failures retain the credential; a rejected refresh removes it and directs the caller to relink. The SDK's OAuth logging target is disabled even under ordinary debug logging because it can include codes and provider responses; application events remain enabled. Account API responses contain metadata only.

OpenAPI sources are deployed with Phase 3. Add `{ "slug": "tickets", "kind": "openapi", "url": "https://service.example/openapi.yaml", "requiresAccount": true }` through the source form or API. JSON and YAML [OpenAPI 3.x documents](https://spec.openapis.org/oas/latest.html) are accepted. Local JSON-pointer references, recursive schemas, path/query/header serialization and JSON request bodies are supported. Tools use `operationId`, falling back to method and path; the JSON body is the `body` keyword argument, with a prefixed name when that collides with a parameter. Read-only request fields are not required merely because they are required in responses.

The effective API server is resolved and saved when adding the source. `serverUrl` can explicitly override it; a document using different operation servers requires that explicit choice. Later document changes cannot move calls to another server. Changing the saved destination removes its linked credential. Both document and operation requests disable redirects; operation requests also disable transport retries and use the configured upstream deadline. Document URLs must be reachable without credentials; linked credentials are sent only to the pinned API server.

Link API credentials with session-protected `PUT /api/accounts/ID/http`. Inputs are `{ "kind": "bearer", "credential": "..." }`, `{ "kind": "api_key_header", "name": "X-Api-Key", "credential": "..." }`, `{ "kind": "api_key_query", "name": "api_key", "credential": "..." }`, or `{ "kind": "basic", "username": "...", "password": "..." }`. Values are encrypted and applied by the broker; account responses remain metadata-only. HTTP errors report their status without returning provider error bodies, and HTTP transport trace logs are suppressed to protect query credentials.

OpenAPI limits are four document loads shared by imports and synchronization, 2 MiB per document, 1,000 operations, a bounded schema/reference compiler, and 8 MiB per UTF-8 response. JSON responses pass through without a value tree, empty responses become `null`, and other UTF-8 responses become strings. External references, cookie parameters and non-JSON request bodies are rejected explicitly. OpenAPI is qualified in the deployed baseline. The local replacement proposal removes execution approvals; live-owner acceptance checks remain recorded in docs/status.md.

Manage workspaces at `/workspaces` or `/api/workspaces`. Create with `{ "name": "Work projects", "sourceIds": ["SOURCE_UUID"] }`; update with `{ "name": "Work projects", "sourceIds": [] }`. The default “All sources” workspace includes all current and future sources and is protected. Custom workspaces include only selected sources; an empty selection exposes no tools. Search, generated Python names, scripts and broker dispatch enforce the token's workspace. Deleting a workspace revokes its tokens while preserving source connections and audit history. Old `/endpoints` bookmarks redirect to `/workspaces`.

Connect all execution clients to `https://codemode.microapps.space/mcp`. OAuth consent requires an explicit workspace choice. Personal tokens require `workspaceId` alongside `name`. There are no custom domains or wildcard routes. The Sources page provides GitHub, Notion, Linear and Sentry presets; adding a preset is followed by browser account linking and explicit provider consent.

The separate `https://codemode.microapps.space/mcp-admin` exposes nine administration tools: `list_workspaces`, `save_workspace`, `delete_workspace`, `list_sources`, `save_source`, `delete_source`, `sync_source`, `list_accounts`, and `source_connection_url`. Use its own OAuth resource and `admin` scope, or issue a personal token with `{ "name": "Management agent", "administration": true }`. Ordinary `/mcp` tokens cannot use management tools, and management tokens cannot execute workspace tools. All management is private to the user's tenant, including for the first account with the admin attribute. Source configuration and credentials are excluded from list results; account linking returns a browser URL. Writes and reads are recorded in the tenant's `administration_calls` audit table with source configuration redacted. A failed audit prevents the operation from starting.

Each resource publishes metadata at `/.well-known/oauth-protected-resource/mcp` or `/.well-known/oauth-protected-resource/mcp-admin`. Its 401 challenge points to that metadata and requests the corresponding `mcp` or `admin` scope.

The local OAuth backend publishes authorization-server metadata on the main hostname, with `/oauth/register`, `/oauth/authorize` and `/oauth/token`. It supports client ID metadata documents first, legacy dynamic registration, public clients and hashed confidential-client secrets using Basic or POST authentication. Client documents require HTTPS, checked and pinned public DNS addresses, no redirects or proxy environment settings, a 32 KB response limit and a matching `client_id`. Shared secrets in client documents are rejected. Registration and uncached discovery share a 30-per-minute limit; discovery permits four concurrent requests and stores at most 1024 clients.

Browser consent uses session-protected `GET /api/oauth/authorize` and `POST /api/oauth/authorize`; approval or denial returns a validated `redirectUri` containing the client's state and the issuer. Agent tokens cannot approve. S256 PKCE and the exact canonical resource URL are required. Codes expire after five minutes and are single-use. Access tokens expire after one hour; opted-in refresh grants last at most 30 days and rotate their tokens. Reusing a code or refresh token revokes its grant, as does owner revocation or workspace deletion. OAuth access tokens use a separate `cmo_` prefix so previous binaries reject them instead of treating them as permanent personal tokens. Codes, refresh tokens, client secrets and access tokens are stored only as hashes.

The `/authorize` React consent screen is deployed and exercised in Chromium against the embedded binary. On Oracle, real Nudge, Researcher and Artifacts discovery, DCR, S256, advertised-resource binding and denied-callback replay checks pass in a disposable database. Successful provider consent/token exchange and claude.ai acceptance remain unperformed.

The owner authorized merging and deploying the access-groups implementation. `tools/deploy.sh` delegates to `tools/deploy.py`, which prepares an exact pushed source revision and selects a private, complete binary/worker bundle with a frozen access mode. The deployment coordinator still needs end-to-end release qualification; see [the access-groups checkpoint](docs/access-groups.md). Deployment keeps the existing access mode until the coordinated ID migration. The existing service environment, key material and backup timer must remain intact.

`python3 tools/smoke.py http://127.0.0.1:2093 codemode.microapps.space TOKEN_FILE` checks a running gateway without printing its token.

The 61 real tool schemas have exact stub snapshots in `docs/trial-stubs.golden`. Regenerate them with `SQLX_OFFLINE=true cargo run -q -p codemode-core --example generate_stubs > docs/trial-stubs.golden` and review the diff.

The pool starts workers with an empty environment. The interpreter never runs inside the gateway process. No host files, network, credentials, import handlers or OS handlers are granted to sandbox Python.

To repeat the source trial, run `python3 tools/trial-fixtures.py` from this directory. It uses the existing sibling application binaries, Postgres and Mongo on `oracle-vm`; creates disposable `codemode_spike_*` databases; and cleans them up on SIGTERM. The Nudge binary is expected at `.tools/bin/nudge-api`. Generated credentials remain under ignored `.tools/trial/` with owner-only permissions. Trial scripts and evidence live in `docs/trial-*.json`.

The gateway runs on **oracle-vm, loopback port 2093, under smdctl**, with a daily PostgreSQL backup timer. Authenticated smoke checks pass both on loopback and through `https://codemode.microapps.space/mcp`. Caddy and DNS have not been changed. The current production catalogue is empty; upstream accounts have not been linked yet.

## Tool policies

Manage tool policies from a workspace's **Manage tool policies** link. Every tool defaults to allow, including destructive tools. A workspace can block a tool or limit its source/tool membership. Blocked tools disappear from search; dispatch rechecks policy, source revision and membership. A blocked call raises Python `PermissionError` naming the tool and workspace before contacting the upstream. Overrides survive synchronization; changing the source destination removes them alongside linked credentials.

Workspace policy APIs are `GET /api/workspaces/{id}/policies?q=...&cursor=...` and `PUT` on the same path with `{ "sourceId": "UUID", "sourceRevision": 0, "upstreamName": "tool", "action": "default|allow|block" }`. Read the current revision from the list before saving. Writes require the tenant's browser session and matching Origin.

The replacement proposal removes approval pages/APIs, paused-run storage, execution snapshots and MCP `resume`. Migration 0018 changes legacy approval overrides to allow and finishes paused executions as failed without replaying upstream side effects. Interrupted running executions also fail on restart. Provider OAuth consent remains a browser action.

## GraphQL sources (Phase 4)

Add a source with connection type **GraphQL endpoint**, its fixed HTTP(S) URL and, when needed, **Require a linked account**. Its account page supports bearer, header/query API key and basic authentication. Changing the destination removes the old credential, policies and tool allowlist entries. Redirects and automatic retries are disabled.

Each source exposes `query` and `mutation` inside its own Python namespace. The root MCP surface remains exactly `search` and `execute`. Use `query` with introspection when you need to discover the upstream schema. The broker parses the document (64 KiB, recursion 64, 16,384 tokens), verifies the selected operation type, and sends only that operation and its reachable fragments. Mutations default to allow and follow workspace policies. Subscriptions are unsupported. Endpoint scopes apply to these two tools; selecting `query` grants query access to the source, rather than individual GraphQL fields.

```python
return await tools.graph.query(
    query="query Item($id: ID!) { item(id: $id) { id name } }",
    variables={"id": "one"},
    operationName="Item",
)
```

`operationName` is required when the document contains multiple operations. GraphQL errors, including partial-data responses, raise catchable `ToolError` with credential-redacted messages. Successful responses return raw `data` without building a JSON value tree; response bodies are limited to 8 MiB. HTTP credentials stay in the host broker.

The Phase 4 production worker entrypoint requires aarch64 Linux, Bubblewrap, the dynamic loader/system libraries mounted by `deploy/worker.sh`, a working systemd user manager with delegated memory/pid controllers, writable `cgroup.kill`/`memory.oom.group`, Python 3 with `os.pidfd_open` and host libseccomp. The trusted `worker-guard.py` watches both the pool-owned launcher and its gateway parent, and reaps the whole worker cgroup on either death; this also covers Bubblewrap's pre-initialization window. `tools/deploy.sh` generates the native filter before activation. Development can use the raw Monty worker; run Oracle qualification with `T_ISOLATED=1` to exercise the production containment profile.

## Saved scripts (Phase 4)

Open an endpoint and choose **Saved scripts** to create, edit or delete reusable Python. Scripts belong to one endpoint: agents find their names/descriptions through `search` and call `execute` with exactly one of `code` or `scriptId`. Search returns at most 20 script matches; refine the query to narrow them. Existing tool pagination is unchanged. Each endpoint can store 128 scripts, each with at most 64 KiB of Python.

Saved scripts use the ordinary execution path, including audit, limits, tool scopes and allow/block policies. A script ID from another workspace is rejected. Editing or deleting a script affects future runs; running executions retain their audited original code. Conflicting browser edits return an error and preserve your draft; copy it before reloading the latest version.

## MCP tasks (Phase 4)

The gateway uses the official rmcp TaskManager for the [SEP-2663 Tasks extension](https://tasks.extensions.modelcontextprotocol.io/seps/2663-tasks-extension). A 2026-07-28 client declares `extensions: {"io.modelcontextprotocol/tasks": {}}` in its per-request client capabilities. Execute then returns `resultType: "task"` and a top-level `taskId`; poll `tasks/get` for the final tool result, or request cooperative cancellation with `tasks/cancel`. Send the standard per-request MCP metadata, `Mcp-Method`, and `Mcp-Name` (tool name for tools/call, task ID for task methods). Clients without the extension keep synchronous tool calls. The root tools remain search and execute.

Handles are bound to the exact access token and hostname, capped at 64 retained tasks, with a five-minute advertised TTL and a two-second suggested poll interval. The SDK retains terminal results for five minutes after completion. The ordinary 60-second execution limit and audit still apply. Task updates do not grant tool access; policies are rechecked on each dispatch.

Task handles/results are held in memory and disappear when the gateway restarts; execution audits remain in Postgres. Running executions are marked interrupted on recovery and are never replayed. Cancellation stops local execution, but an upstream call already sent may have completed; inspect the execution log before repeating a side effect.


### Microapps ID login (default off)

`CODEMODE_ID_ENABLED=false` retains existing password login and ignores unused Microapps ID settings. Enabling it requires `CODEMODE_ID_CLIENT_ID`, `CODEMODE_ID_CLIENT_SECRET` and `CODEMODE_ID_COOKIE_KEY` (64 hex characters of independent random key material, never the upstream credential encryption key). `CODEMODE_ID_ISSUER` defaults to `https://id.microapps.space`; the callback is exactly `https://CODEMODE_HOSTNAME/auth/callback`. Issuer discovery must succeed before background jobs or listening. The SDK exposes `/auth/login?return_to=/local/path`, `/auth/callback` and exact-origin POST `/auth/logout`. ID mode uses a separate host-only browser cookie and closes local password registration/login. The login page shows only a "Sign in with Microapps ID" link in ID mode (`/api/config` reports `idEnabled`). Production enables it through the VM service environment; the Identity client is registered as `Codemode` with that callback. See [rollout preparation](docs/identity-rollout.md).

The shared internal Rust SDK requires Rust 1.100.0; it currently resolves from the sibling `../identity` checkout until a published immutable pin is available. Dependency checks retain the SDK's public-verification-only RSA advisory assessment and reject new direct dependents of the affected RSA version. No RSA 0.9 private signing or decryption is permitted.
