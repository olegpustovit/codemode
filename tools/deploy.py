#!/usr/bin/env python3
"""Build exact committed source, preserve private config, atomically select a bundle."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
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


def desired_mode():
    value = os.environ.get("CODEMODE_DEPLOY_ACCESS_ENABLED")
    if value is None:
        return None
    if value not in ("true", "false"):
        raise RuntimeError("CODEMODE_DEPLOY_ACCESS_ENABLED must be literal true or false")
    return value == "true"


def remote(request):
    helper = (ROOT / "deploy/access_release.py").read_text()
    operations = (ROOT / "deploy/remote_access.py").read_text()
    # One Python stdin payload; no host helper installation or shell sourcing.
    script = "import json,sys,types\nhelper=types.ModuleType('access_release')\n"
    script += "exec(" + repr(helper) + ",helper.__dict__)\nsys.modules['access_release']=helper\n"
    script += "namespace={'__name__':'codemode_remote'}\nexec(" + repr(operations) + ",namespace)\n"
    script += "try:\n result=namespace['main'](json.loads(" + repr(json.dumps(request)) + "))\n print(json.dumps(result))\n"
    script += "except Exception as error:\n print(str(error) if isinstance(error,RuntimeError) else 'remote release operation failed: '+type(error).__name__,file=sys.stderr)\n sys.exit(1)\n"
    result = subprocess.run(["ssh", REMOTE, "python3", "-"], input=script, text=True, capture_output=True, check=False)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "remote release operation failed")
    try:
        return json.loads(result.stdout)
    except ValueError:
        raise RuntimeError("remote release response was invalid") from None


def inspection(access):
    facts = remote({"action": "facts"})
    legacy = facts.get("legacy")
    if legacy:
        hint = legacy.pop("hint")
        if not isinstance(hint, str) or not re.fullmatch(r"[a-f0-9]{7,40}", hint):
            raise RuntimeError("running legacy build cannot be resolved to an immutable commit")
        sha = output(["git", "rev-parse", "--verify", hint + "^{commit}"])
        if not sha.startswith(hint) or not re.fullmatch(r"[a-f0-9]{40}", sha):
            raise RuntimeError("running legacy source commit could not be verified")
        legacy["sha"] = sha
    state = remote({"action": "inspect", "legacy": legacy, "access": access})
    return {**state, "legacy": legacy, "health": facts["health"]}


def main():
    parser = argparse.ArgumentParser()
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--inspect", action="store_true", help="read-only running version/mode/configuration inspection")
    group.add_argument("--rollback", action="store_true", help="restore previous complete bundle, preserving enforcement")
    parser.add_argument("--eval-gate", default=os.environ.get("CODEMODE_EVAL_GATE") or None, metavar="EVALS_REPO", help="after activation run EVALS_REPO/evals/gate.py; roll back if it fails")
    args = parser.parse_args()
    access = desired_mode()
    state = inspection(None if args.rollback else access)
    if args.inspect:
        print(json.dumps(state, sort_keys=True))
        return 0
    if args.rollback:
        result = remote({"action": "rollback", "token": state["token"], "legacy": state["legacy"]})
        print(json.dumps(result, sort_keys=True))
        return 0
    sha = exact_source()
    schema = max(int(path.name.split("_", 1)[0]) for path in (ROOT / "crates/core/migrations").glob("*.sql"))
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
    nonce = uuid.uuid4().hex
    created = remote({"action": "upload", "nonce": nonce, "token": state["token"], "legacy": state["legacy"]})
    directory = created["directory"]
    if not isinstance(directory, str) or not directory.endswith("/incoming/" + nonce) or not directory.startswith("/home/snowbear/.local/share/codemode/"):
        raise RuntimeError("remote upload path was invalid")
    for name, path in UPLOADS.items():
        subprocess.run(["scp", "-q", str(path), REMOTE + ":" + directory + "/" + name], check=True)
    exact_source(sha)
    if checksums != {name: checksum(path) for name, path in UPLOADS.items()}:
        raise RuntimeError("release artifact changed during upload")
    result = remote({"action": "activate", "nonce": nonce, "token": state["token"], "legacy": state["legacy"], "files": checksums, "sha": sha, "schema": schema, "access": access})
    print(json.dumps(result, sort_keys=True))
    if args.eval_gate:
        # Opt-in eval gate: the evals run against the live build, so a failure restores the previous bundle.
        gate = subprocess.run([sys.executable, str(Path(args.eval_gate) / "evals/gate.py")], cwd=args.eval_gate)
        if gate.returncode != 0:
            rollback = remote({"action": "rollback", "token": state["token"], "legacy": state["legacy"]})
            print(json.dumps({"evalGate": "failed", "rollback": rollback}, sort_keys=True))
            raise RuntimeError("eval gate failed; previous release restored")
        print(json.dumps({"evalGate": "passed"}, sort_keys=True))
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
