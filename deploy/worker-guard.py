#!/usr/bin/python3
"""Reap a worker's cgroup when its launcher or gateway dies, including during namespace setup."""
import os
from pathlib import Path
import select
import subprocess
import sys


def main():
    group = next(line[3:] for line in Path('/proc/self/cgroup').read_text().splitlines() if line.startswith('0::'))
    if not Path(group).name.startswith('run-') or not group.endswith('.scope'):
        raise RuntimeError('worker guard requires its own transient scope')
    cgroup = Path('/sys/fs/cgroup') / group.lstrip('/')
    # An OOM must kill the guard and worker together, never the watcher alone.
    (cgroup / 'memory.oom.group').write_text('1')
    kill_fd = os.open(cgroup / 'cgroup.kill', os.O_WRONLY | os.O_CLOEXEC)
    parent_fd = os.pidfd_open(os.getpid())
    owner_pid = os.getppid()
    owner_fd = os.pidfd_open(owner_pid)
    if owner_pid == 1 or os.getppid() != owner_pid:
        raise RuntimeError('worker owner exited during startup')
    if os.fork() == 0:
        # A watcher must not keep the protocol or its caller's SSH output pipes open.
        for fd in (0, 1, 2, 3):
            os.close(fd)
        poll = select.poll()
        poll.register(parent_fd, select.POLLIN)
        poll.register(owner_fd, select.POLLIN)
        poll.poll()
        os.write(kill_fd, b'1')
        os._exit(1)
    os.close(parent_fd)
    os.close(owner_fd)
    os.close(kill_fd)
    status = subprocess.call(sys.argv[1:], env={}, pass_fds=(3,))
    # Do not wait for the watcher: it wakes on this exit, including SIGKILL from the pool.
    os._exit(status if status >= 0 else 128 - status)


if __name__ == '__main__':
    main()
