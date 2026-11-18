# Codemode v7 aggregation update

Production runs `d163eb67183f74ea8fc86549f75754a607eb8b81` on schema 25. The gateway code is on main. The eval harness for this release is local commit `8af27138f4bad066e9c6c8d1831ee38eaced348c` in the sibling repository, which has no remote. Its source, receipts, and all new Codex transcripts are included in the research archive.

The Monty prelude adds `pick(rows, name="Backlog")`. It returns the first exact match or raises `LookupError` with up to six options. Printing a list with at least 20 records shows the first three and a summary of hidden rows and fields. The sampling applies to ordinary Python containers and JSON strings up to 256 KiB. The execute description instructs Codex to compute aggregates in Python.

Replaying the same code from `dd72ff` reduced stdout from 23,720 to 3,404 bytes. The specific histogram passed three of three targeted Codex reruns, and six additional attempts passed the two 1,000-row tasks. The combined nine runs had a median of 2 calls and no final execute stdout over 8 KiB. These are targeted checks. The previous report retains the full Direct MCP, E5, v6, and v7 comparison.

The production gate passed all eight standard smoke tasks and both aggregation checks. Failed or missing aggregation answers fail the gate on the first attempt. The gate tracks UTF-8 bytes from the final gateway execute before the answer. Local and hardened ARM suites passed 158 checks apiece. The running binary matched `a48b3a14d7f76f0749bea37c7f9f63dda19475402d5408ec58848c9be0dd7549`, restored preflight preserved 28 tables, and the public UI and backup timer passed.

[Report](https://artifacts.microapps.space/a/yxxv6fzadtd5) · [Research archive](https://artifacts.microapps.space/a/kz929se0te1s) · [Release receipt](../measurements/aggregation-guard-release.json).
