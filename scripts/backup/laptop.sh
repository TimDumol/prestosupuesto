#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
# Optional private deployment settings, excluded from Git.
if [[ -r "$ROOT/.env.deploy.local" ]]; then
  # foxguard: ignore[bash/source]
  . "$ROOT/.env.deploy.local"
fi
: "${PRESTO_SSH_HOST:?Set PRESTO_SSH_HOST in .env.deploy.local or the environment}"
: "${PRESTO_SSH_USER:?Set PRESTO_SSH_USER in .env.deploy.local or the environment}"
: "${PRESTO_SSH_KEY:?Set PRESTO_SSH_KEY in .env.deploy.local or the environment}"
BASE="$ROOT/.backup"
RESTIC="$BASE/bin/restic"
export RESTIC_PASSWORD_FILE="$BASE/password"
export RESTIC_CACHE_DIR="$BASE/cache"
export RESTIC_REPOSITORY="$BASE/laptop-repository"
mkdir -p "$BASE"
exec 9>"$BASE/laptop.lock"
flock -w 300 9
TEMP=$(mktemp -d "$BASE/pull.XXXXXX")
trap 'rm -rf "$TEMP"' EXIT
ssh -i "$PRESTO_SSH_KEY" -o BatchMode=yes -o ConnectTimeout=15 "$PRESTO_SSH_USER@$PRESTO_SSH_HOST" sudo -n /usr/local/sbin/presto-backup bundle > "$TEMP/bundle.tar"
tar -xf "$TEMP/bundle.tar" -C "$TEMP"
"$RESTIC" -r "$TEMP/repository" check
[[ -f "$RESTIC_REPOSITORY/config" ]] || "$RESTIC" init
"$RESTIC" copy --from-repo "$TEMP/repository" --from-password-file "$RESTIC_PASSWORD_FILE"
"$RESTIC" forget --host athena --tag presto --keep-within 7d --keep-daily 30 --keep-weekly 12 --keep-monthly 12
"$RESTIC" check
date -u +%s > "$BASE/laptop-last-success"
