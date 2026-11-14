#!/usr/bin/env bash
set -euo pipefail
umask 077
cd /home/snowbear/.local/share/codemode
mkdir -p backups
archive="backups/codemode-$(date -u +%Y%m%dT%H%M%SZ).dump"
trap 'rm -f "$archive.tmp"' EXIT
pg_dump -Fc "${CODEMODE_DATABASE_URL:?}" > "$archive.tmp"
mv "$archive.tmp" "$archive"
find backups -name 'codemode-*.dump' -type f -mtime +14 -delete
