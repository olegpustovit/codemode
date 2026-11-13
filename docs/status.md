# Implementation status

Source: [implementation plan, revision 5](https://artifacts.microapps.space/a/saqdh2xm7jcd), read on 2026-11-13.

## Owner decisions

- Shared use: other people will use the gateway; Phase 4 is in scope before shared deployment.
- Main hostname: `codemode.microapps.space`. The pattern for additional endpoint hosts still needs clarification at Phase 2.
- Control-plane sign-in: standalone.
- Trial sources: Nudge, Researcher, Artifacts, using disposable databases and test identities. The implementing model writes the Python.
- Deployment target: `oracle-vm`, via smdctl. Port `2093` was free at discovery; recheck before binding. The owner will configure Caddy.

## Phase gates

| Phase | Current evidence | Remaining |
|---|---|---|
| 0 | Working worker-pool adapter, signed snapshots, dual-era stateless MCP, qualification tests and source trial | Owner reading [the completed report](spike.md); all nine criteria pass on Oracle and the corrected-fixture trial completes 24/24 tasks |
| 1 | Not started | All core-loop requirements, database contracts, hot-path budgets, deployment, daily-use acceptance |
| 2 | Not started | Admin API, standalone sign-in, encrypted accounts, upstream OAuth, agent OAuth, endpoints, embedded Vite UI, claude.ai acceptance |
| 3 | Not started | OpenAPI, policy, browser approvals, persisted single-use resume, expiry and authorization checks |
| 4 | Target VM probes permit unprivileged user/network namespaces and a systemd user scope with MemoryMax | OS isolation, seccomp, per-worker cgroups, rate limits, scopes, GraphQL, saved scripts, MCP tasks |

The application is not deployed yet. No Caddy or DNS settings have been changed. The Phase 0 echo server is a protocol probe, not the completed gateway.
