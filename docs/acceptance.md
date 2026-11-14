# Acceptance ledger

Checked 2026-11-14 against [revision 5 of the implementation plan](https://artifacts.microapps.space/a/saqdh2xm7jcd), the current sources, recorded qualification and the running Oracle process. Implementation is deployed; the owner-dependent gates below remain open. Continuing past a manual gate was authorized; that authorization is not evidence the gate passed.

## Running release

- Service: `smdctl-codemode.service`; daily backup: `smdctl-codemode-task-backup.timer`. Both active at the latest read-only check.
- URL: https://codemode.microapps.space; listener `127.0.0.1:2093`; schema 13.
- Running executable SHA-256: `8d674fef37e9954929a1ef9c19c7276b94bddbc6ead4c31ff7f18656d555a62c`, matching the qualified optimized candidate.
- Production inventory: zero sources, accounts, named endpoints, saved scripts and paused runs. No trial identity or credential was copied into production.
- Final evidence: [measurements and source hashes](measurements/oracle-phase4-final.json), [deployment record](status.md#phase-4-deployed--current-authoritative-status). Local raw logs are under `target/phase4-*` and excluded from Git.

## Gates that prevent full acceptance

| Plan requirement | Current evidence | Evidence still needed |
|---|---|---|
| Phase 1: Claude Code connected and owner uses it daily instead of direct MCP connections | Authenticated public HTTPS calls pass for both protocol eras; execution/audit fixtures pass | Owner's actual client connection and daily-use confirmation |
| Phase 2: successful consent, token exchange and refresh with providers the owner uses | Real Nudge/Researcher/Artifacts discovery, DCR, S256, resource binding and denial/replay checks pass. Controlled providers cover successful exchange, encrypted persistence, concurrent refresh and rotation | Actual owner consent and a successful call/refresh with the real provider account; production currently has no linked account |
| Phase 2: wildcard routing alongside the existing explicit sites | [Additive snippet](../deploy/Caddyfile.wildcard) previously adapted alongside all 20 explicit handlers; Caddy/DNS left unchanged as requested | Owner applies routing; an actual named endpoint resolves publicly and publishes its own resource metadata |
| Phase 2: claude.ai connects to a subdomain through OAuth and calls a browser-linked account | Gateway OAuth/browser/protocol integration checks pass | Actual claude.ai session, subdomain, consent and successful upstream tool call |
| Phase 3: destructive live-source call waits for the owner's click and completes | Real Monty, Postgres and browser fixtures prove pause, owner decision, restart recovery, exactly-once continuation and OpenAPI dispatch | Owner's live-source approval and result |

Owner workflow: sign in to the control plane, add the intended real source and link its account, then select it in a named endpoint. Apply the wildcard snippet alongside existing sites. Connect the client to that endpoint's displayed `/mcp` URL and complete OAuth. Run a read-only call, then a deliberately chosen approval-required call; confirm the browser decision is required before its continuation. Record provider/client, endpoint, execution ID and outcome here or in `docs/status.md`. Daily use needs the owner's confirmation over real use; a synthetic smoke run cannot establish it.

The browser password is held on Oracle at `/home/snowbear/.local/share/codemode/.admin-password` with mode 0600. No password, bearer value or provider credential belongs in this ledger.

## Phase 0 criteria

All nine original criteria have measured passing results in [the spike report](spike.md#measured-criteria). The owner has read that report. Current pool checks live in [pool.rs](../crates/core/tests/it/pool.rs); the throwaway spike crate is retired.

| # | Criterion | Evidence |
|---|---|---|
| 1 | Fifty gathered 100 ms calls overlap | Historical measured 141.9 ms at fan-out 50; current `gather_yields_all_fifty_calls_in_one_batch` and dispatch concurrency checks |
| 2 | Infinite loop stopped in 100 trials | Current hardened extended run: 100/100 stopped at 20 ms turn deadline; worst total 79.245 ms includes setup |
| 3 | Memory excess ends only its worker | Memory-limit pool test plus unchanged kernel cgroup probe; neighbouring execution survives |
| 4 | Deep recursion ends only its worker | Recursion pool test confirms termination and neighbour/subsequent results |
| 5 | Killed worker produces a clean error and replacement | `killed_worker_is_reported_and_replaced` |
| 6 | Ten thousand runs leave gateway memory flat | Current soak: 400 KiB growth, below unchanged 2,048 KiB ceiling; full RSS series in final measurements |
| 7 | Warm start under 5 ms on arm64 | Current maximum 2,001 µs over 100 samples; median 341 µs, p95 473 µs |
| 8 | Snapshot restores in a new process; tampering rejected | Historical cross-process proof; current snapshot test and optimized-binary browser workflow with an actual gateway restart while paused |
| 9 | Same endpoint serves both protocol eras | Gateway integration plus deployed public smoke for 2025-11-25 and 2026-07-28 |

## Implementation and invariant evidence

“Qualified” below means the named automated/technical behavior passed. It does not imply the owner-dependent gates above passed.

| Requirement | Authoritative implementation / qualification evidence | Status |
|---|---|---|
| Rust workspace, thin binary composition, Tokio/axum, typed `CODEMODE_` configuration, exact Monty/rmcp pins | [Cargo.toml](../Cargo.toml), [toolchain](../rust-toolchain.toml), [bootstrap](../crates/gateway/src/bootstrap.rs), [configuration](../crates/gateway/src/config.rs), dependency checks | Qualified |
| Embedded forward-only Postgres migrations and compile-time checked SQL | [Migrations](../crates/core/migrations), `.sqlx/` metadata, SQLx preparation check, restored-copy deployment preflight and live schema 13 | Qualified |
| Rust test runner and build budgets | [Runner](../tools/t/src/main.rs); 2 s unit / 5 s integration deadlines unchanged; final native 87/87, hardened Oracle 87/87 fast and 90/90 extended; core/gateway incremental rebuilds 4.040/2.440 s | Qualified |
| Exactly search, execute and resume; model-facing Python instructions; both MCP eras | [Server](../crates/gateway/src/server.rs), [hostname/protocol integration](../crates/gateway/tests/it/endpoints.rs), deployed smoke | Qualified |
| Import config once; manage sources in Postgres; HTTP/stdio clients, sync and source failure reporting | [Sources](../crates/core/src/components/sources.rs), [source contracts](../crates/core/tests/it/sources.rs), [remote integration](../crates/core/tests/it/remote.rs), [stdio integration](../crates/core/tests/it/stdio.rs) | Qualified |
| Schema-to-Python stubs, sanitization, optional/union/nested/ref handling, Any fallback and real-schema goldens | [Generator and tests](../crates/core/src/components/catalogue/stubs.rs), [61-tool corpus](trial-schemas.json), [goldens](trial-stubs.golden) | Qualified |
| Search ranking, compound names, brief/full output, maximum 20 tools, cursor and permission filtering | [Catalogue](../crates/core/src/components/catalogue.rs), [repository search contract](../crates/core/tests/it/main.rs), [scope integration](../crates/gateway/tests/it/endpoints.rs) | Qualified |
| Authenticate, source/call/time limits, fail-fast execution permits, bounded concurrent dispatch, truncation and cancellation | [Execution](../crates/core/src/components/execution.rs), service/real-pool tests, deployed error/truncation smoke | Qualified |
| Broker argument validation before dispatch, result mapping, catchable errors, redaction and audit | [Broker](../crates/core/src/components/broker.rs), remote/stdio/linked/OpenAPI/GraphQL integration and [audit tests](../crates/core/tests/it/main.rs) | Qualified |
| Monty only in separate workers; no worker secrets/network/host files; validated frames and caller-bound reuse | [Adapter](../crates/core/src/infrastructure/sandbox.rs), pool isolation/caller tests, [kernel probe](../crates/core/tests/isolation/main.rs) and [worker entrypoint](../deploy/worker.sh) | Qualified |
| Host-side credentials; never forward an agent token; authenticated encryption and credential-free API/error/audit results | [Accounts](../crates/core/src/components/accounts.rs), linked/OpenAPI/GraphQL/upstream OAuth tests, [browser account boundary](../crates/gateway/tests/it/upstream_accounts.rs) | Qualified |
| Signed, expiring, same-caller/same-host, single-use paused snapshots; no replay; approval only in browser | [Approvals](../crates/core/src/components/execution/approvals.rs), [approval integration](../crates/core/tests/it/approvals.rs), browser restart/decision workflow and task-update forgery test | Qualified |
| Token creation/revocation and hashing; hostname audiences | [Auth](../crates/core/src/components/auth.rs), token repository/CLI tests, endpoint integration and public challenges | Qualified |
| PKCE, client metadata documents, legacy registration, resource binding, rotation/replay rejection, endpoint deletion and reserved names | [OAuth contracts](../crates/core/tests/it/oauth.rs), [browser OAuth](../crates/gateway/tests/it/authorization.rs), endpoint tests, [reserved inventory](../deploy/reserved-hosts.txt) | Qualified |
| Standalone browser sign-in, protected owner API, exact Origin checks and metadata-only credential responses | [Browser API tests](../crates/gateway/tests/it/browser.rs), session tests, deployed Secure/HttpOnly/SameSite and logout check | Qualified |
| build-react-stack frontend in ui/, React Compiler, TS7, typed routes/query state, Tailwind/shadcn, same-origin API/config | [UI manifest](../ui/package.json), [Vite configuration](../ui/vite.config.ts), [typed queries](../ui/src/features/queries.ts), type/lint checks and five optimized-binary browser workflows | Qualified |
| Rust binary embeds built UI; backend-only builds do not invoke Bun; staging detects stale/modified output; bounded bundles and manifest/icons | [Embedding build](../crates/gateway/build.rs), [staging](../ui/scripts/stage.ts), size check, web-HTTP and deployed asset/deep-link smoke; UI fingerprint 5fc24f83f212 | Qualified |
| Design/response states and responsive control plane | [Design system](../DESIGN.md), [sidecar](../.impeccable/design.json), scoped finish verdicts in `.impeccable/review/`, current browser workflows and inspected deployed desktop/mobile captures | Qualified within incumbent system; no external comp/quality-bar authority claimed |
| OpenAPI 3.x JSON/YAML, local refs and parameter/body handling, pinned destination, four credential forms, no redirects | [OpenAPI integration](../crates/core/tests/it/openapi.rs), browser creation/link/execute workflow | Qualified |
| Per-endpoint allow/approve/block defaults, blocked search/dispatch filtering, policy editing and browser approval page | [Policies](../crates/core/src/components/policies.rs), approval/scope tests and browser policy/approval workflows | Qualified |
| Namespace/seccomp/cgroup worker isolation, 64 MiB cap and startup/owner-death cleanup | Worker wrapper, [guard](../deploy/worker-guard.py), [seccomp](../tools/worker-seccomp.py), isolated Oracle suites; kernel probe covers eight launcher kills and one owner kill with stdin held open; isolation hashes unchanged | Qualified |
| Per-token/per-endpoint rate limits and endpoint tool scopes | [Limiter](../crates/core/src/components/auth/rate_limit.rs), endpoint/approval integration, browser scope persistence | Qualified |
| GraphQL query/mutation separation, selected operation validation, variables/fragments, credentials, errors and approval | [GraphQL integration](../crates/core/tests/it/graphql.rs), browser workflow | Qualified |
| Endpoint-owned saved-script CRUD/search/execute, optimistic revisions, bounds and frozen paused code | [Script contracts](../crates/core/tests/it/scripts.rs), saved-script approval test, browser CRUD/conflict/both-era workflow | Qualified |
| Official MCP task extension, caller-bound bounded handles, polling/cancellation, compatibility and no approval bypass | [Task wrapper](../crates/gateway/src/server/tasks.rs), endpoint task integration, deployed public task smoke | Qualified; handles/results are process-local |
| Three measured hot-path allocation budgets | Catalogue: at most five for paginated row serialization. MCP result: at most 40 for the measured 1,000-object fixture, constant with payload object count. Worker codec: zero warm encode allocations, bounded decode; protected by tests in catalogue/upstream/sandbox modules | Qualified |
| Optimized release, gateway plus worker deployment under smdctl, available loopback port, backups and public availability | [Deploy helper](../tools/deploy.sh), [manifest](../deploy/smdctl.yml), backup/preflight/health logs, matching live SHA and active service/timer | Qualified on 2093 |

## Explicit limits of the evidence

MCP task handles/results reset on a gateway restart. Execution audits and signed approval snapshots persist; interrupted work is not replayed. The first security-schema upgrade's forward-repair failure branch was not fault-injected. Controlled-provider consent/refresh and browser fixtures establish implementation behavior, not successful authentication to the owner's real accounts. Historical failed/interrupted/over-budget runs remain in the status record and are not counted as passes.

## Email/password accounts and private tenancy

Delivered at codemode.microapps.space, schema 14. First-account registration and the admin attribute, restricted email registration, private MCPs/OAuth accounts/credentials, agent tokens, endpoint scopes, scripts and history pass automated qualification. Both tenants can use the same MCP namespace; browser IDs and root-host agent/OAuth access remain tenant-bound. All 89 native and 89 hardened Oracle checks pass, as do six optimized-binary browser workflows. Public registration is verified without creating an account. The first real registration belongs to the owner. Existing agent credentials were revoked by migration. Email verification and password recovery are outside this implementation. See the current account evidence in status.md and .impeccable/review/registration-verdict.md.

## Workspaces and administration MCP (2026-11-14)

The owner replaced custom domains with named workspaces selected during OAuth on the shared `/mcp` URL, requested popular source presets, and authorized a separate `/mcp-admin` management surface. Existing acceptance items about wildcard Caddy/subdomain setup are superseded; no DNS or Caddy change is required.

Local browser qualification covers workspace creation and source membership, token issuance and revocation, mandatory OAuth workspace selection, selected-source preview, old `/endpoints` bookmark redirects, a Notion preset followed by browser account linking, separate administration consent, admin PAT execution and resource separation, both MCP eras, and existing OpenAPI/GraphQL approval and saved-script flows. Tests run in disposable databases with local providers; they create no production users or source connections and do not authorize real providers.

`/mcp-admin` authorizes management only within the signed-in user's tenant. Later users receive the same own-tenant management access; the first user's admin attribute grants no bypass. Source configuration is redacted in management audits, list results exclude credentials, and stdio creation is operator-only. Live upstream OAuth and external-client acceptance remain owner/provider checks; record their outcome without inventing completion or blocking the authorized implementation.
