#!/bin/sh
set -eu
ulimit -c 0
[ "$#" -eq 1 ] && [ "$1" = subprocess ] || { echo 'expected Monty subprocess transport' >&2; exit 2; }
worker_dir=$(CDPATH= cd -- "$(/usr/bin/dirname -- "$0")" && pwd)
[ "$(/usr/bin/uname -m)" = aarch64 ] || { echo 'isolated worker requires aarch64 Linux' >&2; exit 1; }
[ -s "$worker_dir/worker.bpf" ] || { echo 'worker seccomp filter is missing' >&2; exit 1; }
export XDG_RUNTIME_DIR="/run/user/$(/usr/bin/id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
# The guard preserves the pool's PID and reaps the whole cgroup even during namespace setup.
exec /usr/bin/systemd-run --user --scope --quiet --collect \
  --property=MemoryMax=67108864 --property=MemorySwapMax=0 --property=TasksMax=16 \
  /usr/bin/env -i /bin/sh -c 'exec 3<"$1"; shift; guard=$1; shift; exec /usr/bin/env -i /usr/bin/python3 -I -S "$guard" /usr/bin/bwrap "$@"' \
  codemode-worker "$worker_dir/worker.bpf" "$worker_dir/worker-guard.py" \
  --unshare-user --unshare-pid --unshare-net --unshare-ipc --unshare-uts --unshare-cgroup \
  --die-with-parent --new-session --clearenv --cap-drop ALL \
  --hostname codemode-worker \
  --ro-bind "$worker_dir/monty" /worker \
  --ro-bind /usr/bin/env /clearenv \
  --ro-bind /lib/ld-linux-aarch64.so.1 /lib/ld-linux-aarch64.so.1 \
  --ro-bind /lib/aarch64-linux-gnu /lib/aarch64-linux-gnu \
  --proc /proc --dev /dev --dir /tmp --chdir /tmp --remount-ro / \
  --seccomp 3 /clearenv -i /worker subprocess
