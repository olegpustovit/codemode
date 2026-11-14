#!/usr/bin/env bash
set -euo pipefail

# Reuse authentication so SSH setup does not consume each case's hard deadline.
ssh() {
  command ssh -o ControlMaster=auto -o "ControlPath=$HOME/.ssh/codemode-ci-%C" -o ControlPersist=60 "$@"
}

# Used by cargo t after a local glibc-2.35 zigbuild; the VM never compiles the application.
destination=/home/snowbear/ci/codemode
if [[ "$1" == --stage ]]; then
  shift
  ssh -o BatchMode=yes -o ConnectTimeout=5 oracle-vm mkdir -p "$destination"
  scp -q target/aarch64-unknown-linux-gnu/debug/codemode "oracle-vm:$destination/codemode"
  scp -q crates/core/tests/fixtures/stdio.py "oracle-vm:$destination/stdio.py"
  scp -q .tools/monty/bin/monty "oracle-vm:$destination/monty"
  for executable in "$@"; do
    scp -q "$executable" "oracle-vm:$destination/$(basename "$executable")"
  done
  if [[ "${T_ISOLATED:-0}" == 1 ]]; then
    ssh -o BatchMode=yes oracle-vm mkdir -p "$destination/isolation"
    scp -q deploy/worker.sh deploy/worker-guard.py tools/worker-seccomp.py .tools/monty/bin/monty "oracle-vm:$destination/isolation/"
    ssh -o BatchMode=yes oracle-vm 'cd /home/snowbear/ci/codemode/isolation && chmod +x worker.sh && python3 worker-seccomp.py worker.bpf'
  fi
  exit 0
fi
executable=$1
shift
case "$executable" in
  "$PWD"/target/aarch64-unknown-linux-gnu/debug/deps/*) ;;
  *) echo 'expected a locally cross-built test executable' >&2; exit 2 ;;
esac
name=$(basename "$executable")
worker_entry=monty
if [[ "${T_ISOLATED:-0}" == 1 ]]; then worker_entry=isolation/worker.sh; fi
for value in "$name" "${T_CASE_DEADLINE_SECS:?}" "$worker_entry" "$@"; do
  [[ "$value" =~ ^[a-zA-Z0-9_:./=-]+$ ]] || { echo 'invalid test runner argument' >&2; exit 2; }
done
ssh -o BatchMode=yes -o ConnectTimeout=5 oracle-vm bash -s -- "$name" "$T_CASE_DEADLINE_SECS" "$worker_entry" "$@" <<'REMOTE'
set -euo pipefail
cd /home/snowbear/ci/codemode
executable=$1
deadline=$2
worker_entry=$3
shift 3
export CODEMODE_MONTY_BIN=$PWD/$worker_entry
export CODEMODE_GATEWAY_BIN=$PWD/codemode
export CODEMODE_STDIO_FIXTURE=$PWD/stdio.py
export DATABASE_URL='postgresql://snowbear@localhost/postgres?host=/var/run/postgresql'
export CODEMODE_RESULTS_DIR=$PWD/measurements
if [[ "$worker_entry" == isolation/worker.sh ]]; then
  export CODEMODE_RESULTS_DIR=$PWD/measurements-isolated
  exec /usr/bin/setpriv --no-new-privs timeout --signal=KILL "${deadline}s" "./$executable" "$@"
fi
# The remote deadline also kills workers if the SSH client or local wrapper disappears.
exec timeout --signal=KILL "${deadline}s" "./$executable" "$@"
REMOTE
