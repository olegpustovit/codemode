#!/usr/bin/env python3
"""Export the native-architecture Monty syscall allowlist using host libseccomp."""
import ctypes as c
import errno
import os
from pathlib import Path
import sys

ALLOW = 0x7FFF0000
ERRNO = 0x00050000

class Compare(c.Structure):
    _fields_ = [("arg", c.c_uint), ("op", c.c_uint), ("mask", c.c_uint64), ("value", c.c_uint64)]


def export(path):
    lib = c.CDLL("libseccomp.so.2", use_errno=True)
    lib.seccomp_init.argtypes = [c.c_uint32]
    lib.seccomp_init.restype = c.c_void_p
    lib.seccomp_release.argtypes = [c.c_void_p]
    lib.seccomp_release.restype = None
    lib.seccomp_syscall_resolve_name.argtypes = [c.c_char_p]
    lib.seccomp_syscall_resolve_name.restype = c.c_int
    lib.seccomp_rule_add_array.argtypes = [c.c_void_p, c.c_uint32, c.c_int, c.c_uint, c.POINTER(Compare)]
    lib.seccomp_export_bpf.argtypes = [c.c_void_p, c.c_int]
    ctx = lib.seccomp_init(ERRNO | errno.EPERM)
    if not ctx:
        raise RuntimeError("cannot initialize seccomp")
    def rule(name, action=ALLOW, comparisons=()):
        number = lib.seccomp_syscall_resolve_name(name.encode())
        if number < 0:
            return
        args = (Compare * len(comparisons))(*comparisons)
        result = lib.seccomp_rule_add_array(ctx, action, number, len(args), args)
        if result != 0:
            raise OSError(-result, f"cannot compile seccomp rule for {name}")
    try:
        # Protocol I/O, dynamic loading, memory, signals and pthread runtime; no networking or host control.
        for name in """read readv write writev close close_range fcntl fstat newfstatat statx lseek pread64
            access faccessat faccessat2 readlink readlinkat getcwd getdents64
            mmap mmap2 mprotect munmap mremap madvise brk msync
            rt_sigaction rt_sigprocmask rt_sigreturn sigaltstack restart_syscall
            futex futex_waitv set_tid_address set_robust_list rseq membarrier
            clock_gettime clock_getres clock_nanosleep nanosleep gettimeofday time
            getpid getppid gettid getuid geteuid getgid getegid getresuid getresgid
            sched_getaffinity sched_yield getrandom uname sysinfo prlimit64 getrlimit
            epoll_create1 epoll_ctl epoll_pwait epoll_pwait2 epoll_wait eventfd2 poll ppoll
            pipe2 dup dup2 dup3 tgkill tkill wait4 waitid getrusage execve exit exit_group""".split():
            rule(name)
        # Read-only opens are sufficient for the loader and /proc/self; no file creation or writes.
        flags = os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC | os.O_APPEND | getattr(os, "O_TMPFILE", 0)
        rule("openat", comparisons=(Compare(2, 7, flags, 0),))  # SCMP_CMP_MASKED_EQ
        rule("open", comparisons=(Compare(1, 7, flags, 0),))
        # glibc falls back to clone when clone3 is unavailable; only thread clones may proceed.
        rule("clone3", ERRNO | errno.ENOSYS)
        namespaces = 0x02000000 | 0x04000000 | 0x08000000 | 0x10000000 | 0x20000000 | 0x40000000 | 0x00020000
        rule("clone", comparisons=(Compare(0, 7, namespaces | 0x00010000, 0x00010000),))
        for option in (3, 15, 21, 39):  # GET_DUMPABLE, SET_NAME, GET_SECCOMP, GET_NO_NEW_PRIVS
            rule("prctl", comparisons=(Compare(0, 4, option, 0),))
        target = Path(path)
        fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        try:
            result = lib.seccomp_export_bpf(ctx, fd)
            if result != 0:
                raise OSError(-result, "cannot export seccomp filter")
        finally:
            os.close(fd)
        if not target.stat().st_size or target.stat().st_size % 8:
            raise RuntimeError("invalid BPF output")
    finally:
        lib.seccomp_release(ctx)

if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: worker-seccomp.py OUTPUT.bpf")
    export(sys.argv[1])
