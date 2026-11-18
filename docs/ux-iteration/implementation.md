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
| Execution return motion | [#8](https://github.com/olegpustovit/codemode/pull/8) | `3824960915c8ac2bcbf33b009ce146d8cef9c497` | 27 |
| Full-resolution backdrop | [#8](https://github.com/olegpustovit/codemode/pull/8) | `6e01178a64632e66280a8bedaf29091dbe3b0779` | 27 |
| Workspace menu close | [#8](https://github.com/olegpustovit/codemode/pull/8) | `0cc97086970c23acd99f68e0463f4f79bbfbff92` | 27 |
| Named motion markers | [#8](https://github.com/olegpustovit/codemode/pull/8) | `cdc5d9bf9fd7d198efc6c2e0848f491e0ef0b801` | 27 |
| Layout follow-up | [#8](https://github.com/olegpustovit/codemode/pull/8) | `fca4df1112c03811692b3b83e73c0a511777a41e` | 27 |
| Mobile rail | [#8](https://github.com/olegpustovit/codemode/pull/8) | `c147bb0fc1278d76710e538868f24bd9477107c4` | 27 |
| Workspace menu and consent | [#8](https://github.com/olegpustovit/codemode/pull/8) | `bbf04fd409598fa8f78490a0298e2d56461a1112` | 27 |
| Trace call fade | [#8](https://github.com/olegpustovit/codemode/pull/8) | `adab4c83ba22ddfae5f9863956822bbd05562bf1` | 27 |
| Focus cursor glow | [#8](https://github.com/olegpustovit/codemode/pull/8) | `0c4f946ce9c9b3cb9688bd3559870453c8047279` | 27 |
| Status and control motion | [#8](https://github.com/olegpustovit/codemode/pull/8) | `6f7ee5b18a72baa86b99b0a794910b1e11021cd7` | 27 |
| Workspace menu guidance | [#8](https://github.com/olegpustovit/codemode/pull/8) | `1bdd8c86aa5335f653b34f3b79aa5199e813a98f` | 27 |

All four restored-backup checks preserved 28 pre-existing tables. Each live Codex gate passed eight smoke tasks and two large aggregation tasks. Wave 4 additionally passed the live agent save and replay task in three MCP calls. Local qualification passed 163 native cases, 39 UI tests and seven targeted optimized-binary Chromium workflows. All 83 public UI files match staging. The initial Wave 4 release used UI fingerprint `19fb5b05d5e3`.

The optimized Chromium run measured 60.6 ms pane click-to-paint p95, 2.20 ms 300-row key-script p95 and 2.40 ms with scope checkboxes. The longest reported task at 4× CPU throttle was 50 ms. A prior instrumented run measured 55 ms. Software WebGL uses a half-resolution backing buffer to reduce rendering load. Desktop and mobile captures cover all eight prototype scenes, with differences in fixture data, displayed state and software-rendered backdrop. Full one-to-one visual parity remains unverified. Provider consent and external Claude Code and Codex connection acceptance remain owner-only.

The Wave 4 follow-up aligns the login and script scope layouts, restores first-time preset setup, and reduces rendering work during list motion. Its release uses UI fingerprint `f6c408fc15ea` and ARM SHA-256 `3e7c235d332c648798ce164258dbf50ef2ec31dca31a0826bca8b5300f7b9ad9`. The restored-backup preflight preserved 28 tables, all 83 public UI assets match staging, and all 11 live Codex cases passed again. The final candidate passed 163 native cases and 39 UI tests. Chromium passed 15/15 workflows against the debug gateway with the production UI bundle. The final optimized run passed all 15 workflows. It measured pane click-to-paint p95 at 72.2 ms and 300-row key-script p95 at 1.77 ms, or 1.79 ms with scope checkboxes. It reported no long tasks at 4× throttle. The preceding full optimized run reported a 53 ms task, and one earlier isolated repeat reached 61 ms, so the strict timing result varies between runs. Software WebGL uses a 40% backing-buffer scale and caps backdrop redraws at 24 per second.

The hue cross-blend release keeps the interface and shader on the same interpolated color during section navigation. It passed 40 UI tests and the targeted motion and Integrations browser workflows, with 64.4 ms pane click-to-paint p95. The optimized binary passed both the motion workflow and the expanded script promotion workflow, which executes the promoted script from a second workspace and checks that workspace’s tool block. The restored-backup preflight preserved 28 tables, all 83 public UI assets matched staging, and all 11 live Codex cases passed. The embedded UI fingerprint is `c3defcd7ba82` and the ARM SHA-256 is `94a514301854e7718b301b8062f569ef211243c17d0073138c72f6a2ad372d02`.

A fixed-phase shader comparison at 1440×900 found 143,419 pixels with at least one channel differing by more than 8 levels between the full-size render and the scaled software fallback. Both used the same shader, hue and animation phase. The software fallback remains a visible difference from the prototype.

The execution return release restores the left-side entry when closing a scoped execution pane. The optimized Chromium Sessions, Integrations and prototype motion workflows passed. The tests inspect the CSS animations that play after each of nine observed transition types. They verify that the authored directional keyframes use at most 12 px horizontal and 8 px vertical translation, with scale and blur absent. The optimized Integrations run measured 65.4 ms pane click-to-paint p95. The restored-backup preflight preserved 28 tables, all 83 public UI assets matched staging and all 11 live Codex cases passed. The embedded UI fingerprint is `23cb8a611356` and the ARM SHA-256 is `47da7342269e1829a9c1460a868ad53c8996ea1d8c259be9b4f7444c469fe157`.

The full-resolution release renders the backdrop in a worker where supported. On software WebGL, the worker admits redraws during browser idle time and waits 100 ms after input. Browsers that cannot start the worker use the existing local renderer. A fixed-phase comparison against the prototype shader produced identical pixel bytes across all five section hues at 1440×900 and 390×844. Public Chromium confirmed the worker painted at both sizes. The full optimized Chromium suite passed 15 workflows. It measured 72.8 ms pane click-to-paint p95, 1.80 ms 300-row key-script p95 and 1.94 ms with scope checkboxes, with no long tasks at 4× throttle. Two repetitions of the performance workflows also passed. The embedded UI fingerprint is `cdf9568b39a6` and the ARM SHA-256 is `2e9902fa43cee182ed39df7a84eb110f0795e4403ee6a6872494f510ca3796e5`.

The workspace menu release closes the menu after selecting a workspace and names the menu snapshot `ws-menu`. The final optimized Chromium run passed all 15 workflows. It measured 83.6 ms pane click-to-paint p95, 1.71 ms 300-row key-script p95 and 1.84 ms with scope checkboxes, with no long tasks at 4× throttle. Reduced-motion checks cover section navigation, pane open, swap and close, filtering, opening the workspace menu, changing workspace, and collapsing and expanding the rail. The capture workflow waits for a painted worker frame at the target viewport and a section hue within one level per channel of the destination hue. These captures contain local fixture data. Full scene parity remains unverified.

Both releases preserved 28 tables in the restored-backup preflight, served all 84 public UI assets byte for byte and passed all 11 live Codex cases. The workspace menu release UI fingerprint is `3fe8fe35829b` and the ARM SHA-256 is `3e6a71c6a2995d6005f5ee7d9f688fe712ea2c088843163708a61693235eaf2d`.

The named motion release captures the cursor and tab underline, uses document-level push and pop to avoid Chromium's scoped prepaint abort, and shows Integrations search syntax on slash-key hover or wildcard input. All 40 UI tests and 15 optimized Chromium workflows passed release qualification. Pane click-to-paint p95 was 85.6 ms. The 300-row keyboard results were 1.75 ms and 1.77 ms with scope checkboxes. The release run reported no long tasks at 4× throttle. A later capture-suite run recorded a 52 ms task. The restored-backup preflight preserved 28 tables, 84 public UI assets matched staging and all 11 live Codex cases passed. UI fingerprint `082830b42dcf`, ARM SHA-256 `0e3ed4fcfaac243ed38ae2ac5311381286408d49811ba1c9f6aad9a9eab78475`.

The fixture capture suite measured app and prototype boxes at both requested sizes. Shared desktop boxes for Sessions, Trace, Script scope and Collapsed rail matched within 1 px. The desktop empty grid contributes a 24 px gap before the empty state. The prototype displays a 32 px mobile rail control row, while the implementation follows the plan's instruction to skip collapse on mobile. The mobile Scripts header is approximately 42 px shorter beyond that rail difference, and its description still uses the prior copy. Consent and workspace-menu dimensions also differ with the current fixtures. Full-scene parity remains open.

The layout follow-up fills the empty grid body and updates the Scripts description to explain workspace and global scope. The desktop empty-state box matches the prototype at 1440×900. The prototype motion and script scope promotion workflows both passed against the debug gateway with the production UI bundle. The mobile rail follows the written plan, leaving a 32 px difference from the prototype. The remaining visual qualification limits are listed above.

The layout release preserved 28 tables in the restored-backup preflight, matched all 84 public UI assets to staging and passed all 11 live Codex cases. UI fingerprint `3069a6b05606`, ARM SHA-256 `ef3ac8704ceb28d61c84d179fe43d97940d73fa1efc16e8eff68b5281fe44361`.

The mobile rail release restores the prototype’s collapse control and Backslash shortcut in the horizontal layout. The rail measures 85 px in both states at 390×844. All eight measured boxes in the empty scene match at 390×844 and 1440×900. The production UI bundle passed all 40 UI tests and all 15 Chromium workflows against the debug gateway, including reduced motion and keyboard expansion on mobile. The restored-backup preflight preserved 28 tables, all 84 public UI assets matched staging and all 11 live Codex cases passed. UI fingerprint `d6a55dc2f4a9`, ARM SHA-256 `ad43ff2f55262458587d3ad6685c7d09f75e43deb4ffbc4592b154476a59c7e9`.

The workspace menu matches the prototype’s desktop position and width at 1440×900. Its mobile position keeps the full popup within the viewport. Consent places the source list directly below the workspace explanation. The menu and consent heights remain dependent on the fixture entries. All 40 UI tests passed. Chromium passed 14 workflows in the suite run. The remaining workflow recorded a 52 ms task at 4× CPU throttle during the ARM build and passed when rerun in isolation after the compile. The restored-backup preflight preserved 28 tables, all 84 public UI assets matched staging and all 11 live Codex cases passed. UI fingerprint `89c66c2230c5`, ARM SHA-256 `121707690331de6b4c6a773a1af775d2643230a803a3cf8110ff043b5091c050`.

## Geometry audit after the workspace menu release

The browser captures now include the Linear detail pane at both viewport sizes. The table counts reference boxes whose x, y, width and height each match within 1 px on Chromium. For the trace scene, the prototype detail pane corresponds to the app’s execution pane on the right. Captures use local fixtures with different text and data.

| Prototype scene | 1440×900 | 390×844 |
| --- | ---: | ---: |
| Sessions | 7/7 | 7/7 |
| Trace | 8/8 | 8/8 |
| Linear detail | 8/8 | 4/8 |
| Empty | 8/8 | 8/8 |
| Workspace menu | 7/8 | 4/8 |
| Consent | 4/5 | 4/5 |
| Script scope | 8/8 | 4/8 |
| Collapsed rail | 7/7 | 7/7 |

The four shell boxes match in every scene. On mobile, the Linear header is 24.78 px taller, the workspace header is 20.80 px shorter and the Scripts header is 20.79 px taller. These differences carry through to the tabs and pane bounds. Consent is 4.49 px shorter on desktop and 99.94 px taller on mobile. The desktop menu position and width match, with a 15.22 px height difference. The mobile menu uses a 366 px width to fit the viewport. This audit measures boxes. Full screenshot and motion parity remain open.

The follow-up comparison uses the prototype’s Acme workspace name for Linear and Scripts and the prototype’s script title for the Scripts breadcrumb. With those fixture labels in place, both scenes match all eight measured boxes at both sizes. The browser motion and script promotion workflows pass with the aligned fixtures. Twelve of the sixteen scene and viewport combinations now match all measured boxes. The four remaining combinations are the workspace menu and consent at both viewport sizes.

The consent capture now exercises a real OAuth grant requested for Acme through the MCP resource URL. The test verifies preselection, opt-in administration and the redirect with an authorization code. In the focused Chromium run, the mobile consent height was 967.28 px against the prototype’s 969.31 px. Desktop measured 745.38 px against 787.58 px. This fixture still uses a locally registered client and an empty workspace. The workspace menu and prototype motion checks also passed in that run.

The workspace menu motion check reads the entrance animation’s active keyframes in Chromium and observes the opacity transition when Escape dismisses the popup. It confirms the prototype’s 320 ms entrance from opacity zero and a 4 px upward offset, followed on dismissal by a 160 ms fade. The reduced-motion branch continues to require zero running animations. This verifies the menu animation parameters and dismissal path. It does not compare every rendered motion frame.

The Chromium transition observer now checks exact directional movement against the prototype. It verifies that the 420 ms animations start at ±8 px vertically or ±12 px horizontally, finish at zero, and use neither scaling nor blur. The Sessions and prototype motion workflows passed with those stricter checks, including execution pane push and pop, section navigation, and detail pane open and swap.

The trace call release uses the prototype’s 420 ms linear opacity transition and retains the eased block-size transition. The desktop trace and prototype motion Chromium workflows passed. The restored-backup preflight preserved 28 tables, all 84 public UI assets matched staging and all 11 live Codex cases passed. UI fingerprint `ed21e0820ee7`, ARM SHA-256 `4852be1246e80a65de7c39729a10d4f857478180e392ca0cc73734ed383c3bad`.

The focus cursor release restores the prototype’s softer midpoint shadow during the 3.2 s pulse. The prototype motion and 300-row control-plane Chromium workflows passed. The restored-backup preflight preserved 28 tables, all 84 public UI assets matched staging and all 11 live Codex cases passed. UI fingerprint `7bdaaa371f2c`, ARM SHA-256 `c1fef973590360fef35bd4c98ba8be7db17766a05c4a5c74a87b9cfbed099dd4`.

The motion workflow observes the Linear row’s active Web Animation immediately after selection. Chromium confirmed that the sweep runs for 700 ms and moves the background from 100% to 0%, matching the prototype’s sweep keyframes. The workflow passes with the released UI, including the existing reduced-motion check.

The status and control motion release restores the workspace notice’s 420 ms fade and matches the prototype’s rail, brand, row, tab and consent checkbox transition declarations. The prototype motion and workspace consent workflows passed. The 300-row workflow recorded a 53 ms task at 4× CPU throttle and passed on rerun. The restored-backup preflight preserved 28 tables, all 84 public UI assets matched staging and all 11 live Codex cases passed. UI fingerprint `7d5c741be474`, ARM SHA-256 `916eaf295173c7e8dc25de1e0f52219af620c24ca9afdf254ebf0f7d73220d21`.

The workspace menu release restores the prototype’s authorization guidance. The focused Chromium workflows and live Codex gate passed. UI fingerprint `56669d6ac5f8`, ARM SHA-256 `5ac8816d2b95c8952b7c616a5413aeeb2df8879f5265802e749486e7518c93b9`.

The focused menu capture now includes All sources, Acme and Personal, matching the prototype’s row count. Desktop placement and width match exactly. The desktop height is 329.09 px against the prototype’s 315.52 px. At 390×844, the gateway keeps all menu controls within the viewport. The motion and script promotion workflows passed together with the three-row fixture.
