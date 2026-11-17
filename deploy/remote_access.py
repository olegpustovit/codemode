"""Existing-service deployment operations, embedded in one owned SSH payload."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import time
import urllib.request
import uuid
from access_release import FILES, ReleaseGuard, digest, private

ROOT = Path.home() / ".local/share/codemode"
PUBLIC = "https://codemode.microapps.space"
UNIT = "smdctl-codemode.service"


def run(args, **kwargs):
    return subprocess.run(args, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120, **kwargs)


def json_http(url):
    if url.startswith("https://"):
        raw = subprocess.check_output(["curl", "--fail", "--silent", "--show-error", "--connect-timeout", "3", "--max-time", "6", "-H", "Cache-Control: no-cache", url], text=True, stderr=subprocess.DEVNULL)
        return json.loads(raw)
    request = urllib.request.Request(url, headers={"Cache-Control": "no-cache"})
    with urllib.request.urlopen(request, timeout=5) as response:
        if response.status != 200:
            raise RuntimeError("release readiness failed")
        return json.load(response)


def running_hash():
    pid = subprocess.check_output(["systemctl", "--user", "show", UNIT, "--property=MainPID", "--value"], text=True).strip()
    if not pid.isdecimal() or int(pid) < 2:
        raise RuntimeError("service has no running process")
    checksum = hashlib.sha256()
    with open("/proc/" + pid + "/exe", "rb") as source:
        for block in iter(lambda: source.read(1048576), b""):
            checksum.update(block)
    return pid, checksum.hexdigest()


def private_environment():
    result = {}
    path = Path.home() / ".config/smdctl/env/codemode.env"
    for line in path.read_text().splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        items = shlex.split(line, comments=True)
        if len(items) != 1 or "=" not in items[0]:
            raise RuntimeError("service environment needs an explicitly reviewed parser")
        name, value = items[0].split("=", 1)
        if not re.fullmatch(r"[A-Z][A-Z0-9_]*", name):
            raise RuntimeError("invalid service environment name")
        result[name] = value
    return result


def facts():
    health = json_http("http://127.0.0.1:2093/healthz")
    pid, checksum = running_hash()
    if health.get("status") != "ok" or not isinstance(health.get("build"), str) or type(health.get("schemaVersion")) is not int:
        raise RuntimeError("existing service readiness is invalid")
    guard = ReleaseGuard(ROOT)
    if guard.metadata():
        current = guard.current()
        if checksum != current["files"]["codemode"]:
            raise RuntimeError("running process differs from selected release")
        if health.get("build") != current["sha"] or health.get("identityAccessEnforced") is not current["access"]:
            raise RuntimeError("running version/mode differs from selected release")
        legacy = None
    else:
        if checksum != digest(ROOT / "bin/codemode"):
            raise RuntimeError("legacy running binary differs from installed file")
        env = private_environment()
        if health.get("identityAccessEnforced", False) is not False or env.get("CODEMODE_ID_ACCESS_ENABLED", "false") != "false":
            raise RuntimeError("legacy access mode cannot be adopted safely")
        legacy = {"hint": health["build"], "schema": health["schemaVersion"], "binary": checksum, "access": False}
    return {"health": {key: health.get(key) for key in ("status", "build", "schemaVersion", "identityAccessEnforced")}, "legacy": legacy}


def guard_for(request):
    guard = ReleaseGuard(ROOT, legacy=request.get("legacy"))
    guard.validate_command()
    return guard


def readiness(value):
    deadline = time.monotonic() + 35
    while time.monotonic() < deadline:
        try:
            local = json_http("http://127.0.0.1:2093/healthz")
            public = json_http(PUBLIC + "/healthz?release_probe=" + uuid.uuid4().hex)
            pid, checksum = running_hash()
            for health in (local, public):
                build = health.get("build", "")
                matches = build == value["sha"] if value["capable"] else isinstance(build, str) and len(build) >= 7 and value["sha"].startswith(build)
                mode = health.get("identityAccessEnforced") if value["capable"] else health.get("identityAccessEnforced", False)
                if health.get("status") != "ok" or not matches or mode is not value["access"] or type(health.get("schemaVersion")) is not int or health["schemaVersion"] < value["schema"]:
                    raise RuntimeError("selected version/schema/mode did not become ready")
            if local["schemaVersion"] != public["schemaVersion"] or checksum != value["files"]["codemode"]:
                raise RuntimeError("public/local/running release differs")
            if value["capable"]:
                # Inspect only selected mode/worker path. Never expose process
                # cookies, encryption keys, database URL or provider credentials.
                raw = Path("/proc/" + pid + "/environ").read_bytes()
                selected = dict(item.split(b"=", 1) for item in raw.split(b"\0") if b"=" in item)
                expected_worker = str(ROOT / "releases" / value["id"] / "worker.sh").encode()
                if selected.get(b"CODEMODE_ID_ACCESS_ENABLED") != str(value["access"]).lower().encode() or selected.get(b"CODEMODE_MONTY_BIN") != expected_worker:
                    raise RuntimeError("running process did not receive frozen release settings")
            run(["systemctl", "--user", "is-active", "--quiet", "smdctl-codemode-task-backup.timer"])
            return {"build": local["build"], "schema": local["schemaVersion"], "access": value["access"]}
        except Exception:
            time.sleep(0.5)
    raise RuntimeError("release failed local/public/PID/schema/mode health checks")


def restart():
    run(["smdctl", "restart", "codemode"])


def psql(database, query):
    return subprocess.check_output(["psql", "-X", "-v", "ON_ERROR_STOP=1", "-d", database, "-Atqc", query], text=True, stderr=subprocess.DEVNULL, timeout=120).strip()


def snapshot(database, tables):
    result = {}
    for table, columns in tables.items():
        if not re.fullmatch(r"[a-z_][a-z0-9_]*", table) or not all(re.fullmatch(r"[a-z_][a-z0-9_]*", column) for column in columns):
            raise RuntimeError("unexpected database identifier in preservation preflight")
        projection = ",".join('"' + column + '"' for column in columns)
        query = 'COPY (SELECT to_jsonb(r)::text FROM (SELECT ' + projection + ' FROM public."' + table + '") r ORDER BY to_jsonb(r)::text) TO STDOUT'
        options = os.environ.get("PGOPTIONS", "") + " -c statement_timeout=120000"
        command = subprocess.Popen(["psql", "-X", "-v", "ON_ERROR_STOP=1", "-d", database, "-c", query], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env={**os.environ, "PGOPTIONS": options})
        checksum = hashlib.sha256()
        for block in iter(lambda: command.stdout.read(1048576), b""):
            checksum.update(block)
        command.stdout.close()
        if command.wait() != 0:
            raise RuntimeError("private preservation snapshot failed")
        result[table] = checksum.hexdigest()
    return result


def preflight(directory):
    # This command intentionally upgrades only the existing Microapps-ID schema,
    # rather than silently reviving destructive historical tenancy migrations.
    version = int(psql("codemode", "SELECT COALESCE(max(version),0) FROM _sqlx_migrations WHERE success"))
    if version < 22:
        raise RuntimeError("review the historical schema upgrade before this access rollout")
    env = private_environment()
    from urllib.parse import urlparse, parse_qs
    configured = urlparse(env.get("CODEMODE_DATABASE_URL", ""))
    if configured.scheme not in ("postgres", "postgresql") or configured.hostname != "localhost" or configured.path != "/codemode" or configured.username != "snowbear" or configured.password or parse_qs(configured.query).get("host") != ["/var/run/postgresql"]:
        raise RuntimeError("review the configured PostgreSQL target before this deployment")
    backups = ROOT / "backups"
    backups.mkdir(mode=0o700, exist_ok=True)
    backup = backups / ("pre-access-" + uuid.uuid4().hex + ".dump")
    run(["pg_dump", "--format=custom", "--file=" + str(backup), "codemode"])
    run(["pg_restore", "--list", str(backup)])
    clone = "codemode_eval_preflight_" + uuid.uuid4().hex
    created = False
    try:
        run(["createdb", clone]); created = True
        run(["pg_restore", "--exit-on-error", "--no-owner", "--dbname=" + clone, str(backup)])
        columns = psql(clone, "SELECT table_name||':'||column_name FROM information_schema.columns WHERE table_schema='public' AND table_name<>'_sqlx_migrations' ORDER BY table_name,ordinal_position")
        tables = {}
        for pair in columns.splitlines():
            table, column = pair.split(":", 1)
            tables.setdefault(table, []).append(column)
        before = snapshot(clone, tables)
        copied = {**os.environ, **env, "CODEMODE_DATABASE_URL": "postgresql://snowbear@localhost/" + clone + "?host=/var/run/postgresql", "CODEMODE_ID_ACCESS_ENABLED": "false"}
        # The candidate's CLI takes the regular startup migration path but does
        # not start source connections, workers, sessions or provider discovery.
        run([str(directory / "codemode"), "token", "list"], env=copied)
        if before != snapshot(clone, tables):
            raise RuntimeError("restored-copy migration changed protected application rows")
        return {"backup": str(backup), "tablesPreserved": len(tables)}
    finally:
        if created:
            if not re.fullmatch(r"codemode_eval_preflight_[a-f0-9]{32}", clone):
                raise RuntimeError("refusing cleanup of an unowned database")
            run(["dropdb", "--if-exists", clone])


def incoming(request):
    nonce = request.get("nonce", "")
    if not isinstance(nonce, str) or not re.fullmatch(r"[a-f0-9]{32}", nonce):
        raise RuntimeError("invalid owned upload identifier")
    return ROOT / "incoming" / nonce


def activate(request, guard):
    directory = incoming(request)
    expected = request.get("files")
    upload_names = set(FILES) - {"worker.bpf"} | {"worker-seccomp.py"}
    if not isinstance(expected, dict) or set(expected) != upload_names:
        raise RuntimeError("incomplete uploaded worker set")
    for name, checksum in expected.items():
        if not isinstance(checksum, str) or not re.fullmatch(r"[a-f0-9]{64}", checksum) or digest(directory / name) != checksum:
            raise RuntimeError("uploaded file checksum mismatch")
        (directory / name).chmod(0o700)
    for name in ("codemode", "monty"):
        header = (directory / name).read_bytes()[:20]
        if header[:6] != b"\x7fELF\x02\x01" or int.from_bytes(header[18:20], "little") != 183:
            raise RuntimeError("expected ARM64 release and Monty worker")
    if subprocess.check_output([directory / "monty", "--version"], text=True).strip() != "monty-runtime 1.5.0":
        raise RuntimeError("unexpected Monty worker version")
    run(["python3", str(directory / "worker-seccomp.py"), str(directory / "worker.bpf")])
    complete = {**{name: checksum for name, checksum in expected.items() if name in FILES}, "worker.bpf": digest(directory / "worker.bpf")}
    preserved = preflight(directory)
    guard.expect(request["token"])
    selected = guard.install(directory, request["sha"], request["schema"], request.get("access"), complete)
    try:
        restart()
        health = readiness(selected)
        guard.owned(selected)
        return {"release": selected["id"], "health": health, "preservation": preserved}
    except Exception:
        previous, mode = guard.rollback_target()
        restored = guard.restore(selected, previous, mode)
        restart(); readiness(restored)
        raise RuntimeError("activation failed; previous complete release restored with access mode preserved")


def main(request):
    os.umask(0o077)
    action = request.get("action")
    if action == "facts":
        return facts()
    guard = guard_for(request)
    if action == "inspect":
        guard.select(request.get("access"))
        return {**guard.state(), "current": guard.current()}
    if action == "upload":
        guard.expect(request["token"])
        directory = incoming(request)
        directory.mkdir(parents=True, mode=0o700, exist_ok=False)
        return {"directory": str(directory)}
    if action not in ("activate", "rollback"):
        raise RuntimeError("unsupported release action")
    lock = ROOT / ".access-deploy.lock"
    descriptor = os.open(lock, os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w") as held:
        private(lock)
        fcntl.flock(held, fcntl.LOCK_EX)
        guard.expect(request["token"])
        if action == "activate":
            return activate(request, guard)
        selected = guard.current()
        previous, mode = guard.rollback_target()
        restored = guard.restore(selected, previous, mode)
        restart()
        return {"release": restored["id"], "health": readiness(restored)}
