# Codemode v7 discovery recovery

The owner requested the four changes in [the plan](https://artifacts.microapps.space/a/nfnvpkejdb81). See the saved `proposal.html`.

- The 3,072-byte manifest reserves the ranked names for all sources that fit, then upgrades them to signatures in rank order across sources. A long signature can fall back to its complete bare name without dropping later sources' names.
- Positive search splits compound identifiers, accepts word prefixes and maps the irregular plural "statuses" to "status". Quoted phrases, exclusions, OR, cursor paging, authorization and policy markers remain. PostgreSQL trigram similarity provides up to three labeled, workspace-scoped suggestions when no first-page match exists. Blocked tools are excluded from suggestions.
- Long raw container prints can carry a bounded `outputShape` alongside unchanged stdout. The existing streaming shape reader also learns one structural skeleton for generic upstream tool schemas from a successful reply. Schema 25 persists key/type hints scoped to source, tool and revision. Stored skeletons omit values and array counts. Ordinary replies stay on the raw-value path.
- The E5 rerun used its exact archived binary (SHA-256 `95a90c44faea63892cb5b5ab47889f4bdadefa6216e58b956566489d034e4ea1`). Its 120 tasks passed on Codex CLI 0.169.0 with `gpt-6.1-sol`, an excluded warm-up, 8 jobs and gateway deferral off. Median 3 MCP calls, 54,597 input tokens and 17.9 s. V7 runs the 42-run Nudge trial before the same full 120-run setup.

The production deploy retains the live smoke gate and restore preflight. The standard and heavy historical reports remain separate evidence. No Claude runs are part of v7.

## Codex measurements

The candidate used Codex CLI 0.169.0 and `gpt-6.1-sol`, with eight jobs, three repeats, gateway deferral off and an excluded warm-up. The 42-run Nudge trial ran before a separate, fresh 120-run standard cohort. Both used the optimized candidate SHA-256 `d7ce924ce850447289d4744e5cf914effc027264a8527fc03f1f2185bda834ce`.

| Full gateway pack | Correct | Median calls | Input tokens | Wall time |
| --- | ---: | ---: | ---: | ---: |
| E5, pinned rerun | 120/120 | 3 | 54,597 | 17.9 s |
| v6 | 120/120 | 3 | 50,128.5 | 18.85 s |
| v7 | 119/120 | 2 | 44,235.5 | 13.75 s |

On 38 shared tasks, the 114-run Direct MCP arm through the Codex host recorded 3.5 calls, 89,516.5 input tokens and 19.8 s at the median. E5 recorded 3 calls, 55,210 tokens and 18.35 s. V6 recorded 3 calls, 52,074.5 tokens and 19.3 s. V7 recorded 2.5 calls, 47,522 tokens and 13.95 s. The full gateway pack adds two tasks the Direct MCP arm omits. Historical raw MCP heavy-pack data remain in the v6 report.

The Nudge trial finished 41/42 with three calls, 48,939 tokens and 14.4 s at the median. The full cohort's Nudge slice cut v6's five calls to two and 83,737 input tokens to 44,235.5. Cross-app calls fell from 6.5 to 3, matching E5. Cross-app tokens remained above E5 at 66,057.5 versus 55,866. Moneybag kept two calls. Its tokens rose from 36,951 to 38,059, and time fell from 14.4 s to 12.4 s.

Both candidate cohorts missed one repeat of `nudge-priority-histogram`. Each captured reply had all 36 issues and the correct status metadata. Codex read the printed rows and reported six priority-three issues where seven qualified. The archive retains both original attempts. All 15 standard safety runs passed.

V7 met two of six standard efficiency targets. Its 11.0% discovery and 17.4% probe rates missed 10% and 5%. Its 13.75 s and 44,235.5 input tokens missed 13 s and 40,000. The 4.35% execute error rate passed the 5% threshold. All 13 failed executes were followed by a successful execute in the same run. Static literal `tools.search` calls in standard traces fell from 182 to 50. Fifteen v7 replies carried a print-shape hint, and eleven exposed the learned-shape label. The cohorts measure the combined changes.

The source passes 157 native and 157 hardened ARM checks. The candidate status queries both ranked `nudge.list_issue_statuses` first, and the misspelling produced a labeled suggestion. A raw Artifacts print of 3,517 bytes carried a shape hint, and subsequent brief search showed the learned shape. See `docs/measurements/v7-standard-verdict.json`, `v7-e5-verdict.json` and `v7-traces.json` for exact calculations.

## Production release

Source `26624ff` was pushed to main and deployed on 2026-11-18 at schema 25. The restored-backup check preserved all 28 protected tables. The live launch gate passed its L1 and L2 regression checks and all eight Codex smoke tasks, including both safety tasks. All 84 public UI files match staging. The release receipt records the live protocols, signatures, shape probe and service checks.

[Comparison report](https://artifacts.microapps.space/a/8a6ctgwz8w38) and [research archive](https://artifacts.microapps.space/a/e424g392xpcr). The deployed source is `26624ff`.
