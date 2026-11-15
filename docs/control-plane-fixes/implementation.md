# Control-plane fixes — implementation and qualification

Authority: https://artifacts.microapps.space/a/jqm3ze7az66e, preserved verbatim in `proposal.html`. All twelve steps are implemented on top of schema 19. The proposal's recommended decisions apply: custom connector names/icons only, old spelling preserved, Sentry connections retained under Custom, 16 line icons plus monogram, glob terms, and server-wide matching selection.

This is an extension of the pinned final-v5 console. Existing fonts, section hues, transparent surfaces, glyph rings, shader and square controls remain the design authority. `DESIGN.md` and its sidecar remain unchanged; their older pane-animation and selection descriptions are historical drift superseded by this request.

| Step | Implementation and evidence |
| --- | --- |
| 1 | Optimized pre-change baseline in `docs/measurements/pane-baseline.json`: 15 samples, 407.9 ms click-to-painted-pane p95. The same real-gateway probe retains the under-50-ms pane and under-16-ms key assertions. |
| 2 | Six presets, no Sentry registry entry or glyph. Store regression proves an existing Sentry connection and credential survive and appear under Custom. |
| 3 | Base UI account menu exposes Account settings, Linked accounts and deliberate Sign out. Duplicate rail footer removed. Keyboard/menu/logout unit checks and real browser workflow pass. Account settings shows actual account metadata; no unrequested password-reset feature. |
| 4 | Shared empty and no-match states suppress headers/cursor. Scripts includes New script and Open Sessions, verified at desktop/mobile. Empty states and their recovery actions also cover Integrations, Workspaces, Tokens, Sessions and session executions. |
| 5 | Additive migration 0020 backfills exact slug spelling and plug icon; rollback-compatible insert trigger. Create/update/PATCH and MCP-admin expose validated display metadata; slug/Python namespace stays stable. Browser creates Acme Billing/card, renames with F2, changes icon, reloads and executes a saved script using the original namespace. Local stdio transport stays operator-managed while its display metadata remains editable. |
| 6 | Pane columns switch immediately; only detail content animates for 140 ms. Explicit opens commit immediately, arrow cursors remain local, URL updates retain deep links/history, integration details prefetch. Sessions also recovers a deep-linked execution beyond its first page. |
| 7 | Grid outer box removed. Focused cursor has an accent bar; unfocused cursor retains a quiet wash. Buttons/inputs retain their focus ring. Real-browser style assertion and axe scans cover focused policies, empty mobile Scripts and destructive confirmation. |
| 8 | One pure shared selection model handles click/range/toggle, Shift arrows, Space, first/second Ctrl/Meta-A, Escape collapse/back, F2, Delete, ARIA selection and narrow-screen Select checkboxes. Unit tests cover anchor/range/toggle and End loading; browser covers six-row range and touch selection. |
| 9 | Bulk panes provide allow/block, include/exclude, custom sync, script deletion and token revocation. Each endpoint validates IDs or query plus exact count, enforces tenant/workspace scope, commits atomically and audits changed items. Script revisions and the original confirmation selection prevent deleting a newer edit; OAuth revocation includes refresh siblings. Mixed-tenant/stale/count rejection and audit tests pass. Safe confirmation receives focus and Escape cancels. |
| 10 | Shared glob fixtures drive browser matcher and real PostgreSQL ILIKE tests: case-insensitive `*`/`?`, whitespace AND, literal `%`, `_`, backslash and Unicode. Search highlights literal spans and shows a focus hint. No regex feature or upload dependency. |
| 11 | Policies, Sessions and inner executions load additional cursor pages on scroll/keyboard, with exact matching counts. Browser loads 300 tools and checks key-script latency with and without inclusion checkboxes. Session counts include exact All/Errors totals independent of loaded pages. |
| 12 | Qualification and deployment receipts are recorded below and in `docs/measurements/control-plane-fixes-qualification.json`. Deployment must pass restored-backup preservation before cutover. No production accounts, fixtures or provider consent may be created. |

## Qualification completed

Hardened Oracle: **111/111** (103.60 s including 76.30 s build/staging; slowest 4.630 s, unchanged hard deadlines). Exhaustive core: **98/98** (85.64 s including 53.35 s build/staging), including 10,000-run memory soak and 100 deadline trials.

Native `cargo t --workspace`: **111/111**, 29.27 s including 24.81 s build; slowest 2.797 s integration, unchanged 2 s unit / 5 s integration deadlines. UI unit suite: **32/32**. TypeScript and Clippy pass; UI lint retains three incumbent stock-component warnings. Dependency advisories, bans, licenses and sources pass using the already installed cargo-deny binary.

The browser command partitions all twelve workflows into two fresh-fixture runs (seven established fixture workflows and five standalone interaction workflows). Every workflow executes exactly once; the existing 90 s case and 180 s suite deadlines remain unchanged. The previous single run exceeded its global deadline under shared-host load.

The fresh visual reviewer requested two corrections: eliminate a rectangle mirrored behind the account menu and preserve bulk selection when Escape cancels confirmation from the destructive button. Both corrections are scored resolved; the reviewer returned **ship for that fix list**, not whole-surface pixel-equality approval. The detector ran once with zero findings.

Earlier browser failures identified and corrected actual issues: search hint intercepting mobile navigation; stale checkbox presentation during scope saves; cursor URL expectations after arrow navigation; hidden desktop grid asserted at mobile width; and an asynchronous React pane commit adding a missed frame. Failed captures and earlier timing runs are not release acceptance. The accepted optimized-binary batches supply release measurements.

Local verification uses an isolated PostgreSQL 18 UTF-8 cluster on loopback 15496 because the documented default socket is absent. Tests create disposable tenant fixtures only in disposable databases.

## Limits

Provider consent, external-client acceptance and M-series shader performance remain owner-only and unperformed. Incremental core/gateway compilation historically exceeds the 10 s / 5 s targets; this work does not claim those budgets pass. Production contains real connected data; migration/deployment must preserve existing IDs, encrypted credentials, workspaces, policies, scripts and audit history.

## Release and deployment receipt — 2026-11-15

All twelve steps are complete. Source commit [`d45e610`](https://github.com/olegpustovit/codemode/commit/d45e6106a3a67a1456975f8421f814cfdb316d70) is pushed to main and deployed on Oracle, loopback 2093, at https://codemode.microapps.space. Schema **20**, build **d45e6106a3a6**, UI **e16dc707d79d**. Running ARM executable SHA-256: `f03d44a766ad41da3f0f1128a61eb7a26be44c14814f040a9958af4e8e7093f2`.

The exact optimized native binary passes all **12 browser workflows** in two accepted batches: seven established flows (1.4m) and five standalone interaction flows (1.7m). No thresholds or deadlines changed. One earlier timing run recorded a 66.1 ms outlier during simultaneous ARM compilation; the unchanged binary passes with compilation idle. Final pane p95 is **46.4 ms** over 15 samples, versus **407.9 ms** before; per-key script p95 is **3.559 ms**. With 300 loaded tools, key-script p95 is **3.244 ms**, or **3.208 ms** with inclusion checkboxes. Three axe WCAG scans report zero violations. Initial JavaScript is 160,410 bytes gzip and CSS 15,757 bytes gzip; lazy chunks remain below 100 KB.

`tools/deploy.sh` backed up production, migrated a restored copy and compared all **22 existing protected tables**, including session/execution/tool-call history, before cutover. Source display fields backfilled correctly. Live metadata counts remain one user, seven sources, five linked accounts, no saved scripts and one workspace; the encrypted account rows are byte-for-byte unchanged. Deployment created no accounts, provider consent or production fixtures. The disposable preflight database was removed.

The running process checksum matches the staged ARM release; service and backup timer are active with NoNewPrivileges and PrivateTmp. Public HTTPS passes both MCP resource metadata/challenges, security/cache/deep-link/manifest checks and byte comparison of all **84 staged files**. Unauthenticated public browser captures at 1440×1000 and 390×844 show the expected fonts, square controls and shader, no horizontal overflow and no page errors. Authenticated product workflows were qualified against disposable local fixtures, not manufactured production data.

The finish reviewer independently scored both listed corrections resolved: the account menu now uses the proposal's near-opaque surface, and both confirmation buttons intercept Escape without discarding selection. The exact shader is unchanged. Incumbent DESIGN.md and `.impeccable/design.json` are preserved; the old pane-animation description is documented drift superseded by this request. No raster assets were added.

Evidence: `docs/measurements/control-plane-fixes-qualification.json`; `target/fixes-{native,oracle-fast,oracle-slow,ui-unit,release-qualification,release-browser-final,deploy,public-smoke,public-verify,public-browser}.log`; captures under `target/ui-browser/` and `.impeccable/review/`. Provider/client/M-series acceptance and historical incremental-build limitations remain as stated above.
