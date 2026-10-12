#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
BASE=/var/lib/presto-backup
export RESTIC_PASSWORD_FILE=/etc/presto-backup/password
export RESTIC_REPOSITORY="$BASE/repository"
export RCLONE_CONFIG=/etc/presto-backup/rclone.conf
mkdir -p "$BASE"
if [[ ${1:-} == status ]]; then
  local_time=0; drive_time=0; drive_enabled=0
  [[ ! -f "$BASE/local-last-success" ]] || read -r local_time < "$BASE/local-last-success"
  [[ ! -f "$BASE/drive-last-success" ]] || read -r drive_time < "$BASE/drive-last-success"
  [[ ! -f /etc/presto-backup/drive-repository ]] || drive_enabled=1
  printf '%s %s %s\n' "$local_time" "$drive_time" "$drive_enabled"
  exit
fi
exec 9>"$BASE/job.lock"
flock -w 300 9
if [[ ${1:-} == bundle ]]; then
  tar -C "$BASE" -cf - repository
  exit
fi
if [[ ${1:-} == prune ]]; then
  restic prune
  if [[ -f /etc/presto-backup/drive-repository ]]; then
    restic -r "$(cat /etc/presto-backup/drive-repository)" prune
  fi
  exit
fi
if [[ ${1:-} == check ]]; then
  restic check --read-data
  if [[ -f /etc/presto-backup/drive-repository ]]; then
    restic -r "$(cat /etc/presto-backup/drive-repository)" check --read-data
  fi
  exit
fi
mkdir -p "$BASE/staging"
STAGE="$BASE/staging"
rm -f "$STAGE/presto.sqlite" "$STAGE/presto.sqlite-wal" "$STAGE/presto.sqlite-shm" "$STAGE/finance.json"
python3 - "$STAGE/presto.sqlite" <<'PY'
import sqlite3, sys
with sqlite3.connect('file:/opt/presto-finance/data/presto.sqlite?mode=ro', uri=True) as source:
    with sqlite3.connect(sys.argv[1]) as target:
        source.backup(target, pages=256, sleep=0.01)
        result = target.execute('PRAGMA integrity_check').fetchall()
        if result != [('ok',)]:
            raise RuntimeError('SQLite backup integrity failed')
PY
# Export from that same immutable snapshot, including every current finance table.
IMAGE=$(docker inspect --format '{{.Image}}' presto-finance-finance-1)
docker run --rm --network none --read-only --user 0 --cap-drop ALL --security-opt no-new-privileges --entrypoint /app/presto-admin -v "$STAGE:/backup" "$IMAGE" finance export-sqlite --database /backup/presto.sqlite --output /backup/finance.json
cp /opt/presto-finance/app/.env "$STAGE/server.env"
cp /opt/presto-finance/app/compose.production.yaml "$STAGE/compose.production.yaml"
docker inspect --format '{{.Config.Image}} {{.Image}}' presto-finance-finance-1 > "$STAGE/application-version.txt"
(cd "$STAGE" && sha256sum presto.sqlite finance.json server.env compose.production.yaml application-version.txt > SHA256SUMS)
[[ -f "$RESTIC_REPOSITORY/config" ]] || restic init
restic backup --host athena --tag presto "$STAGE"
# Keep the frozen cutover source in a permanently tagged snapshot.
if [[ ! -f "$BASE/cutover-saved" ]]; then
  restic backup --host athena --tag permanent /opt/presto-finance/data/cutover-source-20261001-a0542249 /opt/presto-finance/data/cutover-finance-20261001.json /opt/presto-finance/data/cutover-finance-postwrite-20261001.json /opt/presto-finance/data/presto-precanonical-20261001T182021Z.sqlite
  touch "$BASE/cutover-saved"
fi
restic forget --host athena --tag presto --keep-within 7d --keep-daily 30 --keep-weekly 12 --keep-monthly 12
date -u +%s > "$BASE/local-last-success"
if [[ -f /etc/presto-backup/drive-repository ]]; then
  DRIVE=$(cat /etc/presto-backup/drive-repository)
  # Destination must be explicitly initialized; never silently replace it.
  restic -r "$DRIVE" copy --from-repo "$RESTIC_REPOSITORY" --from-password-file "$RESTIC_PASSWORD_FILE"
  restic -r "$DRIVE" forget --host athena --tag presto --keep-within 7d --keep-daily 30 --keep-weekly 12 --keep-monthly 12
  date -u +%s > "$BASE/drive-last-success"
fi
