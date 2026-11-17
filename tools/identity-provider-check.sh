#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
identity_repo=${IDENTITY_REPO:-../identity}
[[ -f "$identity_repo/e2e/run.mjs" ]] || { echo 'The sibling Identity checkout is required' >&2; exit 1; }
export SQLX_OFFLINE=true
cargo build --locked -p codemode-gateway
make -C "$identity_repo" e2e-build
ID_E2E_TLS=1 ID_E2E_SCENARIO="$PWD/tools/identity-provider-scenario.mjs" node "$identity_repo/e2e/run.mjs"
