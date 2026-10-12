#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
BASE="$ROOT/.backup"
export RESTIC_PASSWORD_FILE="$BASE/password"
export RESTIC_REPOSITORY="${PRESTO_DRILL_REPOSITORY:-$BASE/laptop-repository}"
export RESTIC_CACHE_DIR="$BASE/cache"
TEMP=$(mktemp -d "$BASE/drill.XXXXXX")
trap 'rm -rf "$TEMP"' EXIT
exec 9>"$BASE/laptop.lock"
flock -w 300 9
"$BASE/bin/restic" check --read-data
"$BASE/bin/restic" dump latest --tag presto /var/lib/presto-backup/staging > "$TEMP/snapshot.tar"
tar --no-same-owner -xf "$TEMP/snapshot.tar" -C "$TEMP"
python3 "$ROOT/scripts/backup/verify-snapshot.py" "$TEMP/var/lib/presto-backup/staging"
if [[ -z ${PRESTO_DRILL_REPOSITORY:-} ]]; then
  "$BASE/bin/restic" prune
  date -u +%s > "$BASE/drill-last-success"
else
  date -u +%s > "$BASE/drive-drill-last-success"
fi
