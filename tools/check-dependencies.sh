#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
# cargo-deny cannot scope an advisory exception to a dependency edge.
parents=$(cargo tree --locked --invert rsa@0.9.10 --depth 1 --prefix none)
[[ "$parents" == $'rsa v0.9.10\nopenidconnect v4.1.0' ]] || {
  echo 'RSA advisory exception dependency boundary changed; reassessment required' >&2
  exit 1
}
cargo deny check
