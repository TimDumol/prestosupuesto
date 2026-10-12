#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
DRILL_DIR="$(mktemp -d /tmp/presto-finance-drill.XXXXXX)"
chmod 700 "$DRILL_DIR"
cleanup() {
  rm -rf "$DRILL_DIR"
}
trap cleanup EXIT

DATABASE="$DRILL_DIR/source.sqlite"
EXPORT="$DRILL_DIR/finance-export.json"
RESTORED="$DRILL_DIR/restored.sqlite"
RESTORED_EXPORT="$DRILL_DIR/restored-export.json"

cd "$ROOT"
cargo run --manifest-path server/Cargo.toml --bin presto-admin -- finance import \
  --input fixtures/canonical-db/finance-export \
  --database "$DATABASE" --commit --allow-test-workbook
cargo run --manifest-path server/Cargo.toml --bin presto-admin -- finance verify \
  --input fixtures/canonical-db/finance-export --database "$DATABASE"
PRESTO_FINANCE_DRILL_DB="$DATABASE" cargo test \
  --manifest-path server/Cargo.toml --bin presto-finance-server \
  finance_write_reconciliation_and_close_drill -- --ignored --nocapture
PRESTO_FINANCE_DRILL_DB="$DATABASE" cargo test \
  --manifest-path server/Cargo.toml --bin presto-finance-server \
  mobile_snapshot_restore_drill -- --ignored --nocapture

cargo run --manifest-path server/Cargo.toml --bin presto-admin -- finance export-sqlite \
  --database "$DATABASE" --output "$EXPORT"
EXPORT_SHA256="$(sha256sum "$EXPORT" | cut -d ' ' -f 1)"
cargo run --manifest-path server/Cargo.toml --bin presto-admin -- finance restore-sqlite \
  --input "$EXPORT" --sha256 "$EXPORT_SHA256" --database "$RESTORED"
cargo run --manifest-path server/Cargo.toml --bin presto-admin -- finance export-sqlite \
  --database "$RESTORED" --output "$RESTORED_EXPORT"
python3 - "$EXPORT" "$RESTORED_EXPORT" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source_file:
    source = json.load(source_file)
with open(sys.argv[2], encoding="utf-8") as restored_file:
    restored = json.load(restored_file)

for field in ("databaseSchemaVersion", "operationWatermark", "tableCounts", "tables"):
    if source[field] != restored[field]:
        raise SystemExit(f"SQLite restore parity failed for {field}.")

print(
    "SQLite export/restore parity passed for "
    f"{len(source['tableCounts'])} finance tables and "
    f"{sum(source['tableCounts'].values())} total rows."
)
PY

printf 'All disposable canonical database drills passed. Temporary data will be removed.\n'
