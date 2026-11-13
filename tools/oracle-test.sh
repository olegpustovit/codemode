#!/usr/bin/env bash
set -euo pipefail

# Used by cargo t after a local glibc-2.35 zigbuild; the VM never compiles the application.
destination=/home/snowbear/ci/codemode-spike
if [[ "$1" == --stage ]]; then
  shift
  ssh -o BatchMode=yes -o ConnectTimeout=5 oracle-vm mkdir -p "$destination"
  scp -q target/aarch64-unknown-linux-gnu/debug/codemode-spike "oracle-vm:$destination/codemode-spike"
  scp -q .tools/monty/bin/monty "oracle-vm:$destination/monty"
  for executable in "$@"; do
    scp -q "$executable" "oracle-vm:$destination/$(basename "$executable")"
  done
  exit 0
fi
executable=$1
shift
case "$executable" in
  "$PWD"/target/aarch64-unknown-linux-gnu/debug/deps/*) ;;
  *) echo 'expected a locally cross-built test executable' >&2; exit 2 ;;
esac
name=$(basename "$executable")
for value in "$name" "${T_CASE_DEADLINE_SECS:?}" "$@"; do
  [[ "$value" =~ ^[a-zA-Z0-9_:./=-]+$ ]] || { echo 'invalid test runner argument' >&2; exit 2; }
done
ssh -o BatchMode=yes -o ConnectTimeout=5 oracle-vm bash -s -- "$name" "$T_CASE_DEADLINE_SECS" "$@" <<'REMOTE'
set -euo pipefail
cd /home/snowbear/ci/codemode-spike
executable=$1
deadline=$2
shift 2
export CODEMODE_MONTY_BIN=$PWD/monty
export CODEMODE_SPIKE_BIN=$PWD/codemode-spike
export CODEMODE_RESULTS_DIR=$PWD/measurements
# The remote deadline also kills workers if the SSH client or local wrapper disappears.
exec timeout --signal=KILL "${deadline}s" "./$executable" "$@"
REMOTE
