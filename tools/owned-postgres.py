#!/usr/bin/env python3
"""Run checks on an owned PostgreSQL 18 cluster, never an inherited database URL.

Usage: tools/owned-postgres.py cargo t --workspace
       tools/owned-postgres.py --prepare cargo sqlx prepare --workspace -- --all-targets
The optional prepare bootstrap uses the already-built binary's startup migrations.
"""
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import sys
import tempfile


def postgres_bin():
    candidates = []
    if os.environ.get("POSTGRES_BINDIR"):
        candidates.append(Path(os.environ["POSTGRES_BINDIR"]))
    candidates.extend(Path(p) for p in os.environ.get("PATH", "").split(os.pathsep))
    for base in (Path("/usr/lib/postgresql"), Path("/nix/store")):
        if base.is_dir():
            candidates.extend(p / "bin" for p in base.iterdir() if base.name != "store" or "-postgresql-" in p.name)
    for candidate in candidates:
        if not all((candidate / name).is_file() for name in ("initdb", "pg_ctl")):
            continue
        version = subprocess.run([candidate / "initdb", "--version"], capture_output=True, text=True, check=True).stdout
        if re.search(r"PostgreSQL\) 18\.", version):
            return candidate
    raise RuntimeError("PostgreSQL 18 initdb/pg_ctl are required")


def main():
    args = sys.argv[1:]
    prepare = bool(args and args[0] == "--prepare")
    if prepare:
        args.pop(0)
    if not args:
        raise RuntimeError("supply the check command")
    repo = Path(__file__).resolve().parent.parent
    pg = postgres_bin()
    child = None
    started = False
    with tempfile.TemporaryDirectory(prefix="codemode-eval-pg-") as directory:
        root = Path(directory)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        def command(name, *values):
            return subprocess.run([str(pg / name), *map(str, values)], check=True, stdout=subprocess.DEVNULL)
        try:
            command("initdb", "-D", root / "data", "--auth-local=trust", "--auth-host=trust", "--username=codemode_eval")
            command("pg_ctl", "-D", root / "data", "-l", root / "postgres.log", "-o", f"-h 127.0.0.1 -p {port} -k {root} -c max_connections=160", "start")
            started = True
            database = f"postgresql://codemode_eval@127.0.0.1:{port}/postgres"
            env = dict(os.environ, DATABASE_URL=database, SQLX_OFFLINE="false" if prepare else "true", CODEMODE_ID_ENABLED="false", CODEMODE_ID_ACCESS_ENABLED="false")
            if prepare:
                binary = repo / "target/debug/codemode"
                if not binary.is_file():
                    raise RuntimeError("build the gateway using its committed offline SQL cache before --prepare")
                subprocess.run([binary, "token", "list"], cwd=repo, env=dict(env, CODEMODE_DATABASE_URL=database, CODEMODE_ID_ENABLED="false", CODEMODE_ID_ACCESS_ENABLED="false"), check=True, stdout=subprocess.DEVNULL)
            # The runner's hard per-case deadlines remain unchanged. Limit test
            # concurrency to the capacity of this one disposable database host.
            if hasattr(os, "sched_getaffinity"):
                allowed = sorted(os.sched_getaffinity(0))[:8]
                os.sched_setaffinity(0, allowed)
            child = subprocess.Popen(args, cwd=repo, env=env, start_new_session=True)
            return child.wait()
        finally:
            if child is not None and child.poll() is None:
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait()
            if started:
                command("pg_ctl", "-D", root / "data", "-m", "immediate", "stop")


if __name__ == "__main__":
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except (RuntimeError, subprocess.CalledProcessError) as error:
        # Commands and database errors may include private data. Keep diagnostics
        # at the operation boundary; child checks own their ordinary output.
        print("owned PostgreSQL check failed: " + type(error).__name__, file=sys.stderr)
        sys.exit(1)
