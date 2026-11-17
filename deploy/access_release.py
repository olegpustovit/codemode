"""Atomic Codemode/worker bundle selection with a frozen central-access mode.

The existing service command remains bin/codemode. Its small dispatcher selects
one private manifest and one complete immutable bundle, for service and CLI alike.
No worker may observe a partially replaced set of files during a live cutover.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import stat
import uuid

FILES = ("codemode", "monty", "worker.sh", "worker.bpf", "worker-guard.py")


def digest(path):
    if not path.exists():
        return None
    if not stat.S_ISREG(path.lstat().st_mode):
        raise RuntimeError("release/configuration path is not a regular file")
    checksum = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            checksum.update(block)
    return checksum.hexdigest()


def private(path):
    metadata = path.lstat()
    if metadata.st_uid != os.getuid() or metadata.st_mode & 0o077 or not stat.S_ISREG(metadata.st_mode):
        raise RuntimeError("release artifact must be private, regular and owned")


def fingerprint(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def write_private(path, content, mode=0o600):
    temporary = path.with_name(path.name + "." + uuid.uuid4().hex)
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        temporary.unlink(missing_ok=True)


def dispatcher():
    # This stable format-1 dispatcher deliberately uses no deployment helper or
    # mutable environment flag to choose a release. It applies to token CLI calls
    # as well as systemd, so an opted-in service cannot be bypassed through CLI.
    return b'''#!/usr/bin/python3
import hashlib,json,os,re,stat,sys
from pathlib import Path
root=Path(__file__).resolve().parent.parent
def checked(path):
 s=path.lstat()
 if not stat.S_ISREG(s.st_mode) or s.st_uid!=os.getuid() or s.st_mode&0o077: raise RuntimeError("private owned release artifact required")
 h=hashlib.sha256()
 with path.open("rb") as f:
  for block in iter(lambda:f.read(1048576),b""): h.update(block)
 return h.hexdigest()
try:
 checked(root/"current.release.json")
 value=json.loads((root/"current.release.json").read_text())
 if set(value)!={"version","id","sha","schema","access","capable","files","entry","configuration"}: raise RuntimeError("invalid release metadata")
 if type(value["version"]) is not int or value["version"]!=1 or not re.fullmatch("[a-f0-9]{32}",value["id"]): raise RuntimeError("invalid release version")
 if type(value["access"]) is not bool or type(value["capable"]) is not bool or value["access"] and not value["capable"]: raise RuntimeError("invalid access mode")
 if not re.fullmatch("[a-f0-9]{40}",value["sha"]) or type(value["schema"]) is not int or not 0<value["schema"]<100000: raise RuntimeError("invalid release identity")
 if not all(isinstance(value[k],str) and re.fullmatch("[a-f0-9]{64}",value[k]) for k in ("entry","configuration")): raise RuntimeError("invalid release fingerprint")
 if checked(Path(__file__))!=value["entry"]: raise RuntimeError("release dispatcher changed")
 bundle=root/"releases"/value["id"]
 s=bundle.lstat()
 if not stat.S_ISDIR(s.st_mode) or s.st_uid!=os.getuid() or s.st_mode&0o077: raise RuntimeError("private owned release directory required")
 if set(value["files"])!={"codemode","monty","worker.sh","worker.bpf","worker-guard.py"}: raise RuntimeError("incomplete worker bundle")
 for name,expected in value["files"].items():
  if checked(bundle/name)!=expected: raise RuntimeError("release artifact changed")
 env=dict(os.environ,CODEMODE_ID_ACCESS_ENABLED=str(value["access"]).lower(),CODEMODE_MONTY_BIN=str(bundle/"worker.sh"))
 os.execve(bundle/"codemode",[str(bundle/"codemode"),*sys.argv[1:]],env)
except Exception:
 print("Codemode release selection failed; inspect private release metadata",file=sys.stderr)
 sys.exit(1)
'''


class ReleaseGuard:
    def __init__(self, root, home=None, legacy=None):
        self.root = Path(root)
        self.home = Path.home() if home is None else Path(home)
        self.entry = self.root / "bin/codemode"
        self.unit = self.home / ".config/systemd/user/smdctl-codemode.service"
        self.env = self.home / ".config/smdctl/env/codemode.env"
        self.dropins = self.unit.with_suffix(".service.d")
        self.legacy = legacy

    def protected(self):
        paths = {"unit": self.unit, "environment": self.env, "config": self.root / "smdctl.yml",
                 "sources": self.root / "sources.toml", "encryption-key": self.root / ".encryption-key",
                 "backup": self.root / "backup.sh",
                 "backup-timer": self.home / ".config/systemd/user/smdctl-codemode-task-backup.timer"}
        if self.dropins.exists():
            paths.update({"dropin:" + path.name: path for path in self.dropins.glob("*.conf")})
        return {name: None if not path.exists() else [digest(path), stat.S_IMODE(path.stat().st_mode)] for name, path in paths.items()}

    def configuration(self):
        return fingerprint(self.protected())

    def validate_command(self):
        if not self.unit.is_file() or not self.env.is_file():
            raise RuntimeError("deployment requires the existing Codemode service and environment")
        starts = [line.partition("=")[2] for line in self.unit.read_text().splitlines() if line.strip().startswith("ExecStart=")]
        if len(starts) != 1 or shlex.split(starts[0]) != [str(self.entry)]:
            raise RuntimeError("custom service command/arguments need a reviewed dispatcher")
        for path in self.dropins.glob("*.conf"):
            if any(line.strip().startswith("ExecStart=") for line in path.read_text().splitlines()):
                raise RuntimeError("competing service command override must be reviewed")
        config = self.root / "smdctl.yml"
        if config.is_file():
            for line in config.read_text().splitlines():
                if line[:1].isspace():
                    continue  # Nested backup-task commands do not select the gateway.
                field, separator, value = line.strip().partition(":")
                if separator and field == "args" and value.strip() != "[]":
                    raise RuntimeError("custom service arguments need a reviewed dispatcher")
                if separator and field == "command" and value.strip().strip("\"'") != str(self.entry):
                    raise RuntimeError("custom service command needs a reviewed dispatcher")

    def metadata(self, prefix="current"):
        path = self.root / (prefix + ".release.json")
        if not path.exists():
            return None
        private(path)
        try:
            value = json.loads(path.read_text())
            valid = isinstance(value, dict) and set(value) == {"version", "id", "sha", "schema", "access", "capable", "files", "entry", "configuration"}
            valid = valid and type(value["version"]) is int and value["version"] == 1
            valid = valid and isinstance(value["id"], str) and re.fullmatch(r"[a-f0-9]{32}", value["id"])
            valid = valid and isinstance(value["sha"], str) and re.fullmatch(r"[a-f0-9]{40}", value["sha"])
            valid = valid and type(value["schema"]) is int and 0 < value["schema"] < 100000
            valid = valid and type(value["access"]) is bool and type(value["capable"]) is bool and (not value["access"] or value["capable"])
            valid = valid and isinstance(value["files"], dict) and set(value["files"]) == set(FILES)
            valid = valid and all(isinstance(d, str) and re.fullmatch(r"[a-f0-9]{64}", d) for d in [*value["files"].values(), value["entry"], value["configuration"]])
        except (ValueError, TypeError, KeyError):
            valid = False
        if not valid:
            raise RuntimeError("invalid private release metadata")
        return value

    def validate(self, value, configuration=True, entry=True):
        bundle = self.root / "releases" / value["id"]
        metadata = bundle.lstat()
        if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
            raise RuntimeError("release directory must be private and owned")
        for name in FILES:
            private(bundle / name)
            if digest(bundle / name) != value["files"][name]:
                raise RuntimeError("release/worker bundle fingerprint mismatch")
        if value["entry"] != hashlib.sha256(dispatcher()).hexdigest() or entry and digest(self.entry) != value["entry"]:
            raise RuntimeError("selected release dispatcher changed")
        if entry:
            private(self.entry)
        if configuration and self.configuration() != value["configuration"]:
            raise RuntimeError("protected service configuration drifted")

    def current(self):
        value = self.metadata()
        if value:
            pending = not value["capable"] and self.legacy and self.legacy["access"] is False and self.legacy["sha"] == value["sha"] and self.legacy["binary"] == value["files"]["codemode"] == digest(self.entry)
            # An interrupted initial adoption can leave the original ELF and a
            # complete legacy bundle selected. It remains default-off and may
            # finish replacing only that verified legacy entry under the lock.
            self.validate(value, entry=not pending)
            return value
        self.validate_command()
        if not self.legacy or set(self.legacy) != {"sha", "schema", "binary", "access"}:
            raise RuntimeError("legacy binary needs a verified running SHA/schema/mode")
        if self.legacy["access"] is not False or not re.fullmatch(r"[a-f0-9]{40}", self.legacy["sha"]) or type(self.legacy["schema"]) is not int or self.legacy["schema"] < 1:
            raise RuntimeError("legacy release cannot prove default-off access mode")
        if digest(self.entry) != self.legacy["binary"]:
            raise RuntimeError("legacy binary changed since inspection")
        files = {name: digest(self.root / "bin" / name) for name in FILES}
        if any(value is None for value in files.values()):
            raise RuntimeError("legacy worker bundle is incomplete")
        return {"version": 1, "id": fingerprint(files)[:32], "sha": self.legacy["sha"], "schema": self.legacy["schema"], "files": files,
                "access": False, "capable": False, "entry": hashlib.sha256(dispatcher()).hexdigest(), "configuration": self.configuration()}

    def state(self):
        current = self.current()
        previous = self.metadata("previous")
        if previous:
            self.validate(previous)
        return {"token": fingerprint({"configuration": self.protected(), "current": current, "previous": previous,
                    "entry": digest(self.entry)}), "access": current["access"], "capable": current["capable"]}

    def expect(self, token):
        if not isinstance(token, str) or not re.fullmatch(r"[a-f0-9]{64}", token) or self.state()["token"] != token:
            raise RuntimeError("deployment state changed since preflight; refusing stale selection")

    def select(self, desired=None):
        if desired is not None and type(desired) is not bool:
            raise ValueError("access mode must be a literal boolean")
        current = self.current()
        access = current["access"] if desired is None else desired
        if access and not current["capable"]:
            raise RuntimeError("install an access-capable default-off baseline before enabling")
        return access

    def save(self, value, prefix="current"):
        write_private(self.root / (prefix + ".release.json"), (json.dumps(value, sort_keys=True) + "\n").encode())

    def bundle(self, directory, sha, schema, access, capable, expected=None):
        if not isinstance(sha, str) or not re.fullmatch(r"[a-f0-9]{40}", sha) or type(schema) is not int or not 0 < schema < 100000:
            raise ValueError("full SHA and positive schema are required")
        if type(access) is not bool or type(capable) is not bool or access and not capable:
            raise ValueError("invalid explicit release mode")
        expected = {name: digest(directory / name) for name in FILES} if expected is None else expected
        if not isinstance(expected, dict) or set(expected) != set(FILES):
            raise RuntimeError("expected hashes do not cover the complete worker bundle")
        identity = uuid.uuid4().hex
        bundle = self.root / "releases" / identity
        bundle.mkdir(parents=True, mode=0o700)
        for name in FILES:
            path = directory / name
            checksum = expected[name]
            if checksum is None:
                raise RuntimeError("uploaded worker bundle is incomplete")
            digest(path)  # Reject symlinks or non-regular upload paths.
            contents = path.read_bytes()
            if hashlib.sha256(contents).hexdigest() != checksum:
                raise RuntimeError("bundle source changed after validation")
            write_private(bundle / name, contents, 0o600 if name == "worker.bpf" else 0o700)
            if digest(bundle / name) != checksum:
                raise RuntimeError("bundle copy changed")
        return {"version": 1, "id": identity, "sha": sha, "schema": schema, "access": access, "capable": capable,
                "files": {name: digest(bundle / name) for name in FILES}, "entry": hashlib.sha256(dispatcher()).hexdigest(), "configuration": self.configuration()}

    def install(self, directory, sha, schema, desired=None, expected=None):
        self.validate_command()
        access = self.select(desired)
        current = self.current()
        candidate = self.bundle(directory, sha, schema, access, True, expected)
        if current["sha"] == sha and current["files"] != candidate["files"]:
            raise RuntimeError("same SHA upload differs from the installed bundle")
        if not self.metadata():
            # Copy the entire legacy worker set before replacing its entrypoint.
            # Its restored command also freezes false against inherited env flags.
            current = self.bundle(self.root / "bin", current["sha"], current["schema"], False, False, current["files"])
            self.save(current)
            write_private(self.entry, dispatcher(), 0o700)
        if not current["capable"] and digest(self.entry) == current["files"]["codemode"]:
            write_private(self.entry, dispatcher(), 0o700)
        self.validate(current)
        self.save(current, "previous")
        # This one atomic file swap selects binary, worker, mode and metadata.
        self.save(candidate)
        return candidate

    def owned(self, value):
        selected = self.metadata()
        if not selected or selected["id"] != value["id"]:
            raise RuntimeError("another release replaced this activation")
        if self.configuration() != selected["configuration"] or digest(self.entry) != selected["entry"]:
            raise RuntimeError("protected service configuration/dispatcher changed")

    def rollback_target(self):
        self.validate_command()
        current = self.metadata()
        if not current:
            raise RuntimeError("no selected release available")
        self.owned(current)
        previous = self.metadata("previous")
        if not previous:
            raise RuntimeError("no previous complete release available")
        self.validate(previous)
        access = current["access"] or previous["access"]
        if access and not previous["capable"]:
            raise RuntimeError("rollback binary cannot preserve enforcement")
        return previous, access

    def restore(self, value, previous, access):
        self.owned(value)
        expected, mode = self.rollback_target()
        if expected != previous or mode is not access:
            raise RuntimeError("rollback target/mode changed")
        restored = dict(previous, access=access)
        self.save(restored)
        return restored
