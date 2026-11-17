# Microapps ID access groups — PR checkpoint

The owner requested a Codemode PR only and explicitly excluded Codemode from the
production rollout on 2026-11-17. This branch is a draft; production remains on
`e9a23bbeb1b42a7e1d773809cc9825391c9c55b6`, schema 22, without central access
enforcement. No production data, files, services, consent or credentials were changed.

`CODEMODE_ID_ACCESS_ENABLED` defaults to `false` and requires the existing
confidential ID configuration when enabled. The immutable published SDK is pinned
at `e6f5fc3c1bc6e00697bc517a5126a7b24983c0fe`; only the gateway enables its OIDC
adapter. Tenant ownership resolves through the private persisted subject binding.
Missing owners/bindings deny, including an ownerless bootstrap tenant. Local roles,
RLS, workspace/resource scopes, policy, PKCE and replay checks remain authoritative.

Fresh checks precede OIDC linking/provisioning/session replacement, token issuance,
OAuth code/refresh consumption, tenant/provider startup and protected requests.
Work retains its original subject, with independent 25-second rechecks and a
2-second total check bound through actual workers, detached MCP tasks, upstream
HTTP/SSE and source synchronization. Source runtimes close their connections and
resume after restoration. Execution and administration outcomes finalize
independently; transport deletion can close an existing provider session without
refreshing credentials. Blocked app logout stays available. CLI denial precedes
bootstrap workspace writes; offline recovery and listing/revocation remain available.

Local evidence:

- Full fast native suite: 127 passing cases; the subsequent CLI bootstrap regression
  passed independently. Existing hard 2/5-second case deadlines were retained.
- Access tier: nine passing cases in 30.58 seconds, including real Monty workers,
  upstream MCP/idle SSE closure, source recovery and subject-binding changes.
- Four HTTP readers completed 1,924 removal and 1,922 outage responses. Pending HTTP
  work stopped in 24,984/24,977 ms; detached tasks and actual work stopped in
  24,989/24,982 ms. Existing cookie/workspace recovery passed.
- Actual HTTPS Identity/Codemode daemons: captured verified invitation, reviewed
  policy, OIDC, cookie/PAT/OAuth/refresh, execution worker, real policy-storage outage,
  blocked logout and explicit off/on restoration passed. No ID devices remained.
- UI lint/typecheck and 34 tests passed; staged UI stayed byte-for-byte current.
  Formatting and dependency checks passed. Clippy completed with existing warnings.
- The ARM64 qualification artifact is 29,033,680 bytes and requires glibc 2.34.
  It predates the final CLI bootstrap correction and is not a deployment artifact.
- Eighteen release-library fault checks passed, including real dispatcher execution,
  inherited flag overrides, incomplete/changed bundles, stale state and rollback.

The proposed deployment path selects the binary, Monty worker, isolation files and
literal access mode through one private manifest. Service and CLI share that frozen
selection. Upload checksums, source/push checks, a lock, configuration fingerprint,
restored-copy preservation check and public/local/PID/mode gates protect cutover.
The existing service environment, keys, unit and backup timer remain in place.
The first access-capable baseline must be installed with enforcement off before an
opt-in that can safely roll back. The legacy snapshot was inspected read-only.

Before merging/releasing: qualify the new deployment coordinator and restored-copy
preflight end-to-end, repeat final source/binary checks and rebuild ARM64 at the
committed SHA. Broader production evaluation is being handled separately. This PR
does not authorize or claim a Codemode deployment.

Reproduction uses owned local databases:

```sh
tools/owned-postgres.py cargo t --workspace
T_SLOW=1 tools/owned-postgres.py cargo t --workspace access_
tools/identity-provider-check.sh
python3 -m unittest discover -s deploy -p test_access_release.py
```

Detailed receipts are in `docs/measurements/access-*-20261117.json`.
