#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
CODEMODE_BUILD_ID=$(git rev-parse --short=12 HEAD)
if [[ -n "$(git status --porcelain)" ]]; then CODEMODE_BUILD_ID+="-dirty"; fi
export CODEMODE_BUILD_ID
bun run --cwd ui stage
SQLX_OFFLINE=true cargo build --release -p codemode-gateway
bun run --cwd ui stage --check
CODEMODE_GATEWAY_BIN="$PWD/target/release/codemode" bun run --cwd ui test:browser
