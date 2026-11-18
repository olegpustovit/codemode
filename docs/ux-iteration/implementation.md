# Codemode UX iteration

[Owner prototype and implementation plan](https://artifacts.microapps.space/a/gema4jxq4n60)

## Wave 1 work in progress

- The desktop rail toggles with its footer control or Backslash and remembers the choice in browser storage. The mobile rail stays horizontal.
- Execution traces put expandable tool calls before Python and output. The waterfall uses recorded start times, and Python tool paths open their first matching call. Printed flat JSON objects can be viewed as a table or raw text.
- Sessions use a list with failure counts and relative activity. Opening a run leaves the session and execution panes visible on desktop. J and K step through runs and update the URL.
- Automatic execution titles group repeated calls in first-use order. An explicit code comment still takes precedence for a run. New sessions show up to three distinct source namespaces in first-use order and a count of additional namespaces. Built-in calls do not contribute. Previously stored titles are preserved because the existing schema does not distinguish historical automatically stored titles from user renames.

- Section navigation uses typed browser view transitions for upward and downward movement. Sessions animate pane open, close, push, pop, swap and tab changes. The focus cursor breathes, opening a row triggers a light pass, and entering a detail pane draws its edge. Reduced motion bypasses the snapshot transitions.
- Empty lists center their content and actions. Search hints appear when a query contains a wildcard.

The focused execution-title and session naming tests passed through `cargo t`. The session test covers repeated namespaces, built-ins, overflow count, and manual rename priority. The UI build, bundle budget, and 36 UI unit tests passed. An earlier browser run stopped at the staged-asset check because the source changed during the optimized build. The optimized desktop trace, OpenAPI, Sessions, and Integrations workflows passed on the prior Wave 1 UI snapshot. Integrations measured 59.7 ms p95 against the prototype’s 100 ms budget. The workspaces header and integration detail refinements still need browser verification on the staged release. Full motion, visual review, and deployment remain pending.
