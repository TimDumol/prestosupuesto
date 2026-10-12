#!/usr/bin/env bash
set -Eeuo pipefail
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
notify() {
  local title=$1 message=$2
  printf '%s: %s\n' "$title" "$message" >&2
  if command -v notify-send >/dev/null; then
    notify-send --urgency=critical "$title" "$message" || true
  else
    busctl --user call org.freedesktop.Notifications /org/freedesktop/Notifications org.freedesktop.Notifications Notify 'susssasa{sv}i' Presto 0 '' "$title" "$message" 0 1 urgency y 2 0 >/dev/null 2>&1 || true
  fi
}
last=0
[[ ! -f "$BASE/laptop-last-success" ]] || read -r last < "$BASE/laptop-last-success"
if (( $(date +%s) - last > 129600 )); then
  notify 'Presto backup overdue' 'No successful laptop backup in the last 36 hours.'
  exit 1
fi
if status=$(ssh -i "$PRESTO_SSH_KEY" -o BatchMode=yes -o ConnectTimeout=15 "$PRESTO_SSH_USER@$PRESTO_SSH_HOST" sudo -n /usr/local/sbin/presto-backup status); then
  read -r server_last drive_last drive_enabled <<< "$status"
  now=$(date +%s)
  if (( now - server_last > 43200 || (drive_enabled == 1 && now - drive_last > 43200) )); then
    notify 'Presto server backup overdue' 'Server or Google Drive backups have not succeeded in 12 hours.'
    exit 1
  fi
else
  notify 'Presto backup status unavailable' 'Cannot reach Athena to check backup freshness.'
  exit 1
fi
