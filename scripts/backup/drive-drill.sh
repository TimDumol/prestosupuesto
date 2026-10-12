#!/usr/bin/env bash
set -Eeuo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
export PATH="$ROOT/.backup/bin:$PATH"
export RCLONE_CONFIG="$ROOT/.backup/rclone.conf"
export PRESTO_DRILL_REPOSITORY=rclone:presto-drive:presto-backup
exec "$ROOT/scripts/backup/laptop-drill.sh"
