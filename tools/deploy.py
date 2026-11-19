#!/usr/bin/env python3
"""Build exact pushed source and deploy the bundle to oracle-vm through Aquifer (see the aquifer-deploy skill)."""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parent.parent
REMOTE = "oracle-vm"
UPLOADS = {"codemode": ROOT / "target/aarch64-unknown-linux-gnu/release/codemode",
           "monty": ROOT / ".tools/monty-aarch64/bin/monty",
           "worker.sh": ROOT / "deploy/worker.sh", "worker-guard.py": ROOT / "deploy/worker-guard.py",
           "worker-seccomp.py": ROOT / "tools/worker-seccomp.py"}


def output(args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def checksum(path):
    value = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1048576), b""):
            value.update(block)
    return value.hexdigest()


def exact_source(expected=None):
    sha = output(["git", "rev-parse", "HEAD"])
    if expected is not None and sha != expected:
        raise RuntimeError("source HEAD changed during release preparation")
    if output(["git", "status", "--porcelain", "--untracked-files=normal"]):
        raise RuntimeError("commit all source changes before deployment")
    branch = output(["git", "symbolic-ref", "--short", "HEAD"])
    published = output(["git", "ls-remote", "--exit-code", "origin", "refs/heads/" + branch]).split()
    if not published or published[0] != sha:
        raise RuntimeError("push the exact source commit before deployment")
    return sha


def ssh(*args):
    subprocess.run(["ssh", "-o", "BatchMode=yes", REMOTE, *args], check=True)


def main():
    parser = argparse.ArgumentParser()
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--inspect", action="store_true", help="show the running release and recent deploys")
    group.add_argument("--rollback", action="store_true", help="restore the previous release")
    parser.add_argument("--eval-gate", default=os.environ.get("CODEMODE_EVAL_GATE") or None, metavar="EVALS_REPO", help="after activation run EVALS_REPO/evals/gate.py; roll back if it fails")
    args = parser.parse_args()
    if args.inspect:
        ssh("aq", "status", "codemode")
        return 0
    if args.rollback:
        ssh("aq", "rollback", "codemode")
        return 0
    sha = exact_source()
    worker = UPLOADS["monty"]
    header = worker.read_bytes()[:20]
    if header[:6] != b"\x7fELF\x02\x01" or int.from_bytes(header[18:20], "little") != 183:
        raise RuntimeError("expected the pinned ARM64 Monty worker")
    worker_sha = checksum(worker)
    env = {**os.environ, "SQLX_OFFLINE": "true", "CODEMODE_BUILD_ID": sha,
           "PATH": str(ROOT / "deploy/toolchain") + os.pathsep + os.environ["PATH"]}
    subprocess.run(["bun", "run", "--cwd", "ui", "stage", "--check"], cwd=ROOT, env=env, check=True)
    subprocess.run(["cargo", "zigbuild", "--locked", "--release", "-p", "codemode-gateway", "--target", "aarch64-unknown-linux-gnu.2.35"], cwd=ROOT, env=env, check=True)
    subprocess.run(["bun", "run", "--cwd", "ui", "stage", "--check"], cwd=ROOT, env=env, check=True)
    exact_source(sha)
    if checksum(worker) != worker_sha:
        raise RuntimeError("Monty worker changed during release compilation")
    checksums = {name: checksum(path) for name, path in UPLOADS.items()}
    directory = "/tmp/codemode-" + sha[:12] + "-" + uuid.uuid4().hex[:8]
    ssh("mkdir", "-m", "700", directory)
    try:
        for name, path in UPLOADS.items():
            subprocess.run(["scp", "-q", str(path), REMOTE + ":" + directory + "/" + name], check=True)
        exact_source(sha)
        if checksums != {name: checksum(path) for name, path in UPLOADS.items()}:
            raise RuntimeError("release artifact changed during upload")
        # The seccomp filter must be compiled with the host's libseccomp.
        ssh("python3", directory + "/worker-seccomp.py", directory + "/worker.bpf")
        ssh("unlink", directory + "/worker-seccomp.py")
        ssh("aq", "deploy", "codemode", directory)
    finally:
        ssh("rm", "-r", "--", directory)
    if args.eval_gate:
        # Opt-in eval gate: the evals run against the live build, so a failure restores the previous release.
        gate = subprocess.run([sys.executable, str(Path(args.eval_gate) / "evals/gate.py")], cwd=args.eval_gate)
        if gate.returncode != 0:
            ssh("aq", "rollback", "codemode")
            raise RuntimeError("eval gate failed; previous release restored")
        print("eval gate passed")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
    except Exception as error:
        print("Codemode deployment failed: " + type(error).__name__, file=sys.stderr)
        sys.exit(1)
