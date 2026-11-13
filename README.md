# Codemode MCP Gateway

Implementation of [the revision 5 plan](https://artifacts.microapps.space/a/saqdh2xm7jcd), one phase at a time.

Phase 0 is implemented: Monty subprocess pool, signed cross-process snapshots, stateless dual-era MCP and a 24-task trial against real Nudge, Researcher and Artifacts binaries with disposable test databases.

See [measured results](docs/spike.md) and [full-scope status](docs/status.md). The owner reading the spike report is the gate to Phase 1. This is not yet the production gateway.

## Development

Requires Rust 1.96.0, uv and `mold`/clang on aarch64 Linux.

```sh
uv venv .tools/monty
uv pip install --python .tools/monty/bin/python pydantic-monty-runtime==1.5.0
cargo t --workspace
T_SLOW=1 cargo t --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo deny check
```

The pool starts workers with an empty environment. The interpreter never runs inside the gateway process. No host files, network, credentials, import handlers or OS handlers are granted to sandbox Python.

To repeat the source trial, run `python3 tools/trial-fixtures.py` from this directory. It uses the existing sibling application binaries, Postgres and Mongo on `oracle-vm`; creates disposable `codemode_spike_*` databases; and cleans them up on SIGTERM. The Nudge binary is expected at `.tools/bin/nudge-api`. Generated credentials remain under ignored `.tools/trial/` with owner-only permissions. Trial scripts and evidence live in `docs/trial-*.json`.

The eventual deployment target is **oracle-vm, loopback port 2093, under smdctl**. Port availability has been checked; it must be checked again at deployment. Caddy and DNS are left to the owner. The spike binaries staged on the VM are qualification artifacts, with no persistent service running.
