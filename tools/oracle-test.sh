#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Reuse authentication so SSH setup does not consume each case's hard deadline.
ssh() {
  command ssh -o ControlMaster=auto -o "ControlPath=$HOME/.ssh/codemode-ci-%C" -o ControlPersist=60 "$@"
}

# Used by cargo t after a local glibc-2.35 zigbuild; the VM never compiles the application.
destination=/home/snowbear/ci/codemode
if [[ "$1" == --stage ]]; then
  shift
# Oracle's worker is ARM64 even when the build machine is x86-64.
monty_bin=.tools/monty-aarch64/bin/monty
python3 - "$monty_bin" <<'ARCH'
from pathlib import Path
import sys
with Path(sys.argv[1]).open('rb') as binary:
    header = binary.read(20)
assert header[:6] == b'\x7fELF\x02\x01' and int.from_bytes(header[18:20], 'little') == 183, 'expected an ARM64 Monty worker'
ARCH
  ssh -o BatchMode=yes -o ConnectTimeout=5 oracle-vm mkdir -p "$destination"
  scp -q target/aarch64-unknown-linux-gnu/debug/codemode "oracle-vm:$destination/codemode"
  scp -q crates/core/tests/fixtures/stdio.py "oracle-vm:$destination/stdio.py"
  scp -q "$monty_bin" "oracle-vm:$destination/monty"
  worker_version=$(ssh -o BatchMode=yes oracle-vm "$destination/monty" --version)
  [[ "$worker_version" == "monty-runtime 1.5.0" ]] || { echo "Expected Monty runtime 1.5.0" >&2; exit 1; }
  for executable in "$@"; do
    scp -q "$executable" "oracle-vm:$destination/$(basename "$executable")"
  done
  if [[ "${T_ISOLATED:-0}" == 1 ]]; then
    ssh -o BatchMode=yes oracle-vm mkdir -p "$destination/isolation"
    scp -q deploy/worker.sh deploy/worker-guard.py tools/worker-seccomp.py "$monty_bin" "oracle-vm:$destination/isolation/"
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
