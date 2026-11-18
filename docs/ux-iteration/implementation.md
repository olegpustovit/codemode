# Codemode UX iteration

[Owner prototype and implementation plan](https://artifacts.microapps.space/a/gema4jxq4n60)

## Wave 1

- The desktop rail toggles with its footer control or Backslash and remembers the choice in browser storage. The mobile rail stays horizontal.
- Execution traces put expandable tool calls before Python and output. The waterfall uses recorded start times, and Python tool paths open their first matching call. Printed flat JSON objects can be viewed as a table or raw text.
- Sessions use a list with failure counts and relative activity. Opening a run leaves the session and execution panes visible on desktop. J and K step through runs and update the URL.
- Automatic execution titles group repeated calls in first-use order. An explicit code comment still takes precedence for a run. New sessions show up to three distinct source namespaces in first-use order and a count of additional namespaces. Built-in calls do not contribute. Previously stored titles are preserved because the existing schema does not distinguish historical automatically stored titles from user renames.

- Section navigation uses typed browser view transitions for upward and downward movement. Sessions animate pane open, close, push, pop, swap and tab changes. The focus cursor breathes, opening a row triggers a light pass, and entering a detail pane draws its edge. Reduced motion bypasses the snapshot transitions.
- Empty lists center their content and actions. Search hints appear when a query contains a wildcard.

The focused execution-title and session naming tests passed through `cargo t`. The session test covers repeated namespaces, built-ins, overflow count, and manual rename priority. The UI build, bundle budget, and 36 UI unit tests passed. An earlier browser run stopped at the staged-asset check because the source changed during the optimized build. The optimized desktop trace, OpenAPI, Sessions, and Integrations workflows passed on the prior Wave 1 UI snapshot. Integrations measured 59.7 ms p95 against the prototype’s 100 ms budget. Subsequent release qualification is recorded below.

## Wave 2

Workspace selection carries through authenticated URLs. The shell menu exposes workspace URLs and workspace actions. Consent preselects a workspace only when the current tenant can see it, and both code exchange and refresh require the grant's canonical resource. Explicit MCP URLs are checked against the token's workspace.

The wave plan anticipated no migration. Schema 25 has no OAuth grant resource field, so this wave adds a nullable resource column in migration 0026. Existing grants retain their plain MCP resource binding. The script and administration migration planned for Wave 4 moves to 0027.

Local UI type checking and 37 unit tests passed before rebasing onto the latest Wave 1 work. The release and live gateway checks passed. Owner-only external client verification remains pending.

## Wave 3 and Wave 4

The Wave 3 namespaced-login browser workflow passed with provider OAuth stubbed. It verifies the created source, workspace membership, and grouped integration row. Search results label tools from providers with multiple connections.

Wave 4 adds migration 0027 for global scripts, provenance, and explicit per-token administration. Saved scripts continue to run under the calling workspace's policies. The separate administration route is retired, and the broker handles its ten operations with the existing administration audit. The eval gate can also require a Codex run that saves a workspace script and replays it by ID.

## Deployed builds

| Wave | Pull request | Production commit | Schema |
| --- | --- | --- | --- |
| 1 | [#5](https://github.com/olegpustovit/codemode/pull/5) | `f4039e3` | 25 |
| 2 | [#6](https://github.com/olegpustovit/codemode/pull/6) | `cdfbf7f` | 26 |
| 3 | [#7](https://github.com/olegpustovit/codemode/pull/7) | `232a388` | 26 |
| 4 | [#8](https://github.com/olegpustovit/codemode/pull/8) | `98b36e8c7f30747bd9cc7b311511deeb989971c3` | 27 |
| 4 follow-up | [#8](https://github.com/olegpustovit/codemode/pull/8) | `a589dbb1061383def58c505cb044ebec39073446` | 27 |
| Hue cross-blend | [#8](https://github.com/olegpustovit/codemode/pull/8) | `97138daea5ceca5547c1e11aeea786f495640867` | 27 |

All four restored-backup checks preserved 28 pre-existing tables. Each live Codex gate passed eight smoke tasks and two large aggregation tasks. Wave 4 additionally passed the live agent save and replay task in three MCP calls. Local qualification passed 163 native cases, 39 UI tests and seven targeted optimized-binary Chromium workflows. All 83 public UI files match staging. The initial Wave 4 release used UI fingerprint `19fb5b05d5e3`.

The optimized Chromium run measured 60.6 ms pane click-to-paint p95, 2.20 ms 300-row key-script p95 and 2.40 ms with scope checkboxes. The longest reported task at 4× CPU throttle was 50 ms. A prior instrumented run measured 55 ms. Software WebGL uses a half-resolution backing buffer to reduce rendering load. Desktop and mobile captures cover all eight prototype scenes, with differences in fixture data, displayed state and software-rendered backdrop. Full one-to-one visual parity remains unverified. Provider consent and external Claude Code and Codex connection acceptance remain owner-only.

The Wave 4 follow-up aligns the login and script scope layouts, restores first-time preset setup, and reduces rendering work during list motion. Its release uses UI fingerprint `f6c408fc15ea` and ARM SHA-256 `3e7c235d332c648798ce164258dbf50ef2ec31dca31a0826bca8b5300f7b9ad9`. The restored-backup preflight preserved 28 tables, all 83 public UI assets match staging, and all 11 live Codex cases passed again. The final candidate passed 163 native cases and 39 UI tests. Chromium passed 15/15 workflows against the debug gateway with the production UI bundle. The final optimized run passed all 15 workflows. It measured pane click-to-paint p95 at 72.2 ms and 300-row key-script p95 at 1.77 ms, or 1.79 ms with scope checkboxes. It reported no long tasks at 4× throttle. The preceding full optimized run reported a 53 ms task, and one earlier isolated repeat reached 61 ms, so the strict timing result varies between runs. Software WebGL uses a 40% backing-buffer scale and caps backdrop redraws at 24 per second.

The hue cross-blend release keeps the interface and shader on the same interpolated color during section navigation. It passed 40 UI tests and the targeted motion and Integrations browser workflows, with 64.4 ms pane click-to-paint p95. The optimized binary passed both the motion workflow and the expanded script promotion workflow, which executes the promoted script from a second workspace and checks that workspace’s tool block. The restored-backup preflight preserved 28 tables, all 83 public UI assets matched staging, and all 11 live Codex cases passed. The embedded UI fingerprint is `c3defcd7ba82` and the ARM SHA-256 is `94a514301854e7718b301b8062f569ef211243c17d0073138c72f6a2ad372d02`.

A fixed-phase shader comparison at 1440×900 found 143,419 pixels with at least one channel differing by more than 8 levels between the full-size render and the scaled software fallback. Both used the same shader, hue and animation phase. The software fallback remains a visible difference from the prototype.
