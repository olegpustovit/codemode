#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Oracle's worker is ARM64 even when the build machine is x86-64.
monty_bin=.tools/monty-aarch64/bin/monty
python3 - "$monty_bin" <<'ARCH'
from pathlib import Path
import sys
with Path(sys.argv[1]).open('rb') as binary:
    header = binary.read(20)
assert header[:6] == b'\x7fELF\x02\x01' and int.from_bytes(header[18:20], 'little') == 183, 'expected an ARM64 Monty worker'
ARCH
export SQLX_OFFLINE=true
CODEMODE_BUILD_ID=$(git rev-parse --short=12 HEAD)
if [[ -n "$(git status --porcelain)" ]]; then CODEMODE_BUILD_ID+="-dirty"; fi
export CODEMODE_BUILD_ID
expected_schema=$(python3 -c 'from pathlib import Path; print(max(int(p.name.split("_", 1)[0]) for p in Path("crates/core/migrations").glob("*.sql")))')
bun run --cwd ui stage
cargo zigbuild --release -p codemode-gateway --target aarch64-unknown-linux-gnu.2.35
bun run --cwd ui stage --check
expected_sha=$(sha256sum target/aarch64-unknown-linux-gnu/release/codemode)
expected_sha=${expected_sha%% *}
ssh oracle-vm bash -s <<'PREPARE'
set -euo pipefail
mkdir -p /home/snowbear/.local/share/codemode/bin
if [[ -f /home/snowbear/.local/share/codemode/smdctl.yml ]]; then
  cp /home/snowbear/.local/share/codemode/smdctl.yml /home/snowbear/.local/share/codemode/smdctl.yml.prev
fi
PREPARE
scp target/aarch64-unknown-linux-gnu/release/codemode oracle-vm:/home/snowbear/.local/share/codemode/bin/codemode.new
scp "$monty_bin" oracle-vm:/home/snowbear/.local/share/codemode/bin/monty.new
scp deploy/worker.sh oracle-vm:/home/snowbear/.local/share/codemode/bin/worker.sh.new
scp deploy/worker-guard.py oracle-vm:/home/snowbear/.local/share/codemode/bin/worker-guard.py.new
scp tools/worker-seccomp.py oracle-vm:/home/snowbear/.local/share/codemode/worker-seccomp.py
scp deploy/smdctl.yml deploy/backup.sh oracle-vm:/home/snowbear/.local/share/codemode/
ssh oracle-vm bash -s -- "$CODEMODE_BUILD_ID" "$expected_schema" "$expected_sha" <<'REMOTE'
set -euo pipefail
umask 077
expected_build=$1
expected_schema=$2
expected_sha=$3
cd /home/snowbear/.local/share/codemode
[[ "$(bin/monty.new --version)" == "monty-runtime 1.5.0" ]] || { echo "Expected Monty runtime 1.5.0" >&2; exit 1; }
# Generate the native-architecture filter before replacing any live worker files.
python3 worker-seccomp.py bin/worker.bpf.new
if [[ ! -f sources.toml ]]; then
  printf 'sources = []\n' > sources.toml
fi
database_exists=$(psql -d postgres -Atqc "SELECT 1 FROM pg_database WHERE datname='codemode'")
if [[ "$database_exists" != 1 ]]; then
  createdb codemode
fi
previous_schema=0
if [[ $(psql -d codemode -Atqc "SELECT to_regclass('public._sqlx_migrations') IS NOT NULL") == t ]]; then
  previous_schema=$(psql -d codemode -Atqc 'SELECT COALESCE(max(version),0) FROM _sqlx_migrations WHERE success')
fi
mkdir -p backups
backup="backups/pre-deploy-$(date -u +%Y%m%dT%H%M%SZ)-$$.dump"
pg_dump --format=custom --file="$backup" codemode
pg_restore --list "$backup" >/dev/null
# Prove the candidate loads and migrates a restored copy before changing the live binary/database.
preflight_db="codemode_preflight_$$"
trap 'dropdb --if-exists "$preflight_db"' EXIT
createdb "$preflight_db"
pg_restore --exit-on-error --no-owner --dbname="$preflight_db" "$backup"
python3 - "$preflight_db" <<'PRESERVE'
import json, os, subprocess, sys
name = sys.argv[1]
tables = ('tenants', 'users', 'sources', 'tools', 'accounts', 'endpoints',
          'endpoint_sources', 'endpoint_tools', 'access_tokens', 'sessions',
          'oauth_clients', 'oauth_grants', 'oauth_codes', 'oauth_tokens',
          'upstream_oauth_states', 'saved_scripts', 'administration_calls', 'tool_policies', 'oauth_apps', 'control_changes',
          'agent_sessions', 'executions', 'tool_calls')
def snapshot(table, legacy=False):
    # Values stay in this process; diagnostics must never print credential rows.
    value = "to_jsonb(r)"
    if table == 'sources' and source_display_added:
        value += " - 'name' - 'icon'"
    if legacy and table == 'tool_policies':
        value += " || jsonb_build_object('action',CASE WHEN action IN ('approve','ask') THEN 'allow' ELSE action END)"
    query = f"SELECT COALESCE(jsonb_agg(v ORDER BY v::text),'[]'::jsonb) FROM (SELECT {value} v FROM {table} r) q"
    return json.loads(subprocess.check_output(['psql', '-X', '-v', 'ON_ERROR_STOP=1', '-d', name, '-Atqc', query], text=True))
present = set(subprocess.check_output(['psql', '-X', '-v', 'ON_ERROR_STOP=1', '-d', name, '-Atqc', "SELECT tablename FROM pg_tables WHERE schemaname='public'"], text=True).splitlines())
tables = tuple(table for table in tables if table in present)
source_columns = set(subprocess.check_output(['psql', '-X', '-d', name, '-Atqc', "SELECT column_name FROM information_schema.columns WHERE table_schema='public' AND table_name='sources'"], text=True).splitlines())
source_display_added = 'name' not in source_columns
before = {table: snapshot(table, True) for table in tables}
env = dict(os.environ, CODEMODE_DATABASE_URL=f'postgresql://snowbear@localhost/{name}?host=/var/run/postgresql')
subprocess.run(['bin/codemode.new', 'token', 'list'], env=env, stdout=subprocess.DEVNULL, check=True)
for table in tables:
    if before[table] != snapshot(table):
        raise SystemExit(f'Preflight migration changed protected table: {table}')
if source_display_added:
    invalid = subprocess.check_output(['psql', '-X', '-d', name, '-Atqc', "SELECT count(*) FROM sources WHERE name IS DISTINCT FROM slug OR icon<>'plug'"], text=True).strip()
    if invalid != '0':
        raise SystemExit('Preflight display-name/icon backfill did not preserve source identity')
print(f'Preflight migration preserved {len(tables)} protected tables (legacy policies normalized; source display fields backfilled).')
PRESERVE
# Keep deployment credentials on this host; never place them in logs or the repository.
if [[ ! -f .encryption-key ]]; then
  python3 -c 'import secrets; print(secrets.token_hex(32))' > .encryption-key
fi
chmod 600 .encryption-key
export CODEMODE_ENCRYPTION_KEY=$(cat .encryption-key)
dropdb "$preflight_db"
trap - EXIT
if [[ -f bin/codemode ]]; then cp bin/codemode bin/codemode.prev; fi
if [[ -f bin/monty ]]; then cp bin/monty bin/monty.prev; fi
for worker_file in worker.sh worker.bpf worker-guard.py; do
  if [[ -f bin/$worker_file ]]; then cp "bin/$worker_file" "bin/$worker_file.prev"; fi
done
mv bin/codemode.new bin/codemode
mv bin/monty.new bin/monty
mv bin/worker.sh.new bin/worker.sh
mv bin/worker.bpf.new bin/worker.bpf
mv bin/worker-guard.py.new bin/worker-guard.py
chmod +x bin/codemode bin/monty bin/worker.sh backup.sh
# This host's smdctl rejects an existing name; preserve its private environment across recreation.
if systemctl --user cat smdctl-codemode.service >/dev/null 2>&1; then
  cp /home/snowbear/.config/smdctl/env/codemode.env .service-env.saved
  smdctl rm -f codemode
fi
restore_environment() {
  if [[ -f .service-env.saved ]]; then
    cat .service-env.saved > /home/snowbear/.config/smdctl/env/codemode.env || return 1
    python3 - <<'PYENV' || return 1
import os
from pathlib import Path
path = Path('/home/snowbear/.config/smdctl/env/codemode.env')
content = '\n'.join(line for line in path.read_text().splitlines() if not line.startswith(('CODEMODE_MONTY_BIN=', 'CODEMODE_ADMIN_PASSWORD_HASH=', 'CODEMODE_SNAPSHOT_KEY=')))
content += '\nCODEMODE_MONTY_BIN=/home/snowbear/.local/share/codemode/bin/worker.sh\n'
existing = {line.partition('=')[0] for line in content.splitlines()}
for name in ('CODEMODE_ENCRYPTION_KEY',):
    if name not in existing:
        content = content.rstrip('\n') + '\n' + name + '=' + os.environ[name] + '\n'
path.write_text(content)
path.chmod(0o600)
PYENV
    smdctl restart codemode || return 1
    rm .service-env.saved
  fi
}
if smdctl run -f smdctl.yml && restore_environment && systemctl --user is-active --quiet smdctl-codemode-task-backup.timer; then
  for attempt in {1..10}; do
    pid=$(systemctl --user show smdctl-codemode.service --property=MainPID --value)
    running_sha=$(sha256sum "/proc/$pid/exe" 2>/dev/null || true)
    if [[ "${running_sha%% *}" == "$expected_sha" ]] && health=$(curl --fail --silent http://127.0.0.1:2093/healthz) && python3 -c 'import json,sys; h=json.loads(sys.argv[1]); assert h["status"]=="ok" and h["build"]==sys.argv[2] and h["schemaVersion"]>=int(sys.argv[3])' "$health" "$expected_build" "$expected_schema"; then
      printf '%s\n' "$health"
      printf '\nCodemode listening on 127.0.0.1:2093\n'
      exit 0
    fi
    sleep 1
  done
fi
current_schema=$(psql -d codemode -Atqc 'SELECT COALESCE(max(version),0) FROM _sqlx_migrations WHERE success')
if (( (previous_schema < 2 && current_schema >= 2) || (previous_schema < 9 && current_schema >= 8) || (previous_schema < 10 && current_schema >= 10) || (previous_schema < 12 && current_schema >= 12) || (previous_schema < 14 && current_schema >= 14) || (previous_schema < 15 && current_schema >= 15) || (previous_schema < 18 && current_schema >= 18) )); then
  echo "Deployment health check failed after an incompatible schema upgrade; the previous binary cannot enforce this schema safely. Keep the candidate for forward repair. Database backup: $backup" >&2
  exit 1
fi
if [[ -f bin/codemode.prev ]]; then
  mv bin/codemode.prev bin/codemode
  if [[ -f bin/monty.prev ]]; then mv bin/monty.prev bin/monty; fi
  for worker_file in worker.sh worker.bpf worker-guard.py; do
    if [[ -f bin/$worker_file.prev ]]; then mv "bin/$worker_file.prev" "bin/$worker_file"; fi
  done
  if [[ -f smdctl.yml.prev ]]; then cp smdctl.yml.prev smdctl.yml; fi
  if ! systemctl --user cat smdctl-codemode.service >/dev/null 2>&1; then smdctl run -f smdctl.yml; fi
  restore_environment
  smdctl restart codemode
fi
echo 'Deployment failed its health check' >&2
exit 1
REMOTE
