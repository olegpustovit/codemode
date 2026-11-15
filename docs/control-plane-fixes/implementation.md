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

## Qualification in progress

Hardened Oracle: **111/111** (103.60 s including 76.30 s build/staging; slowest 4.630 s, unchanged hard deadlines). Exhaustive core: **98/98** (85.64 s including 53.35 s build/staging), including 10,000-run memory soak and 100 deadline trials.

Native `cargo t --workspace`: **111/111**, 29.27 s including 24.81 s build; slowest 2.797 s integration, unchanged 2 s unit / 5 s integration deadlines. UI unit suite: **32/32**. TypeScript and Clippy pass; UI lint retains three incumbent stock-component warnings. Dependency advisories, bans, licenses and sources pass using the already installed cargo-deny binary.

The browser command partitions all twelve workflows into two fresh-fixture runs (seven established fixture workflows and five standalone interaction workflows). Every workflow executes exactly once; the existing 90 s case and 180 s suite deadlines remain unchanged. The previous single run exceeded its global deadline under shared-host load.

The fresh visual reviewer requested two corrections: eliminate a rectangle mirrored behind the account menu and preserve bulk selection when Escape cancels confirmation from the destructive button. Correction verification is in progress. The detector ran once with zero findings.

Earlier browser failures identified and corrected actual issues: search hint intercepting mobile navigation; stale checkbox presentation during scope saves; cursor URL expectations after arrow navigation; hidden desktop grid asserted at mobile width; and an asynchronous React pane commit adding a missed frame. Failed captures and earlier timing runs are not release acceptance. The final optimized-binary run supplies release measurements.

Local verification uses an isolated PostgreSQL 18 UTF-8 cluster on loopback 15496 because the documented default socket is absent. Tests create disposable tenant fixtures only in disposable databases.

## Limits

Provider consent, external-client acceptance and M-series shader performance remain owner-only and unperformed. Incremental core/gateway compilation historically exceeds the 10 s / 5 s targets; this work does not claim those budgets pass. Production contains real connected data; migration/deployment must preserve existing IDs, encrypted credentials, workspaces, policies, scripts and audit history.
