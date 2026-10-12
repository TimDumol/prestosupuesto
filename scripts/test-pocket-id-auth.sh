#!/usr/bin/env bash
# Build and run a disposable localhost instance, then wait for a real Pocket ID callback.
set -Eeuo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${PRESTO_SERVER_ENV_FILE:-$ROOT/.env.server.local}"
CLIENT_ID="${PRESTO_POCKET_ID_CLIENT_ID:-presto-presupuesto-web}"
TIMEOUT_SECONDS="${PRESTO_AUTH_TEST_TIMEOUT_SECONDS:-600}"
CONFIG_MARKER="$ROOT/.pocket-id-client-configured.json"
RESULT_MARKER="$ROOT/.pocket-id-auth-tested.json"

for command in cargo curl jq npm python3 rg; do
  command -v "$command" >/dev/null || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 127
  }
done
[[ -r "$ENV_FILE" ]] || {
  printf 'server environment file is missing: %s\n' "$ENV_FILE" >&2
  exit 2
}
[[ -r "$CONFIG_MARKER" ]] || {
  printf 'run scripts/configure-pocket-id.sh before testing login.\n' >&2
  exit 2
}
jq -e --arg id "$CLIENT_ID" '.clientId == $id' "$CONFIG_MARKER" >/dev/null || {
  printf 'configured Pocket ID client does not match %s.\n' "$CLIENT_ID" >&2
  exit 2
}

set -a
# The environment file is local, Git-ignored, and mode 0600.
# foxguard: ignore[bash/source]
. "$ENV_FILE"
set +a
jq -e --arg subjects "$POCKET_ID_ALLOWED_SUBJECTS" \
  '.allowedSubjects == ($subjects | split(","))' "$CONFIG_MARKER" >/dev/null || {
  printf 'configured Pocket ID subjects do not match the local server environment.\n' >&2
  exit 2
}
[[ "${PUBLIC_ORIGIN:-}" == 'http://localhost:3000' ]] || {
  printf 'localhost auth test requires PUBLIC_ORIGIN=http://localhost:3000.\n' >&2
  exit 2
}
[[ "${POCKET_ID_REDIRECT_URI:-}" == 'http://localhost:3000/auth/pocket-id/callback' ]] || {
  printf 'localhost auth test requires the localhost Pocket ID callback.\n' >&2
  exit 2
}
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/server/target}"
SERVER_BINARY="$CARGO_TARGET_DIR/release/presto-finance-server"

temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/presto-pocket-test.XXXXXX")"
server_pid=''
cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" >/dev/null 2>&1 || true
    wait "$server_pid" >/dev/null 2>&1 || true
  fi
  rm -rf "$temp_dir"
}
trap cleanup EXIT

if [[ ! -d "$ROOT/web/node_modules" ]]; then
  npm install --prefix "$ROOT/web"
fi
npm run --prefix "$ROOT" web:build
cargo build --manifest-path "$ROOT/server/Cargo.toml" --release

export BIND_ADDR=127.0.0.1:3000
export DATABASE_PATH="$temp_dir/presto.sqlite"
"$SERVER_BINARY" > "$temp_dir/server.log" 2>&1 &
server_pid=$!

ready=0
for _ in $(seq 1 60); do
  if curl --fail --silent --show-error --max-time 2 http://localhost:3000/healthz >/dev/null 2>&1; then
    ready=1
    break
  fi
  if ! kill -0 "$server_pid" >/dev/null 2>&1; then
    printf 'local finance server stopped before it became ready.\n' >&2
    tail -n 40 "$temp_dir/server.log" >&2
    exit 1
  fi
  sleep 1
done
[[ "$ready" == 1 ]] || {
  printf 'local finance server did not become ready.\n' >&2
  tail -n 40 "$temp_dir/server.log" >&2
  exit 1
}

rm -f "$RESULT_MARKER"
printf 'Open http://localhost:3000 and complete Pocket ID sign-in as tim or jopi.\n'
printf 'Waiting up to %s seconds for Pocket ID to return to the localhost callback...\n' "$TIMEOUT_SECONDS"
username=''
for _ in $(seq 1 "$TIMEOUT_SECONDS"); do
  username="$(rg --no-line-number --only-matching 'username=(tim|jopi)' "$temp_dir/server.log" | tail -n 1 | cut -d= -f2 || true)"
  if [[ -n "$username" ]]; then break; fi
  if ! kill -0 "$server_pid" >/dev/null 2>&1; then
    printf 'local finance server stopped before Pocket ID authentication completed.\n' >&2
    tail -n 40 "$temp_dir/server.log" >&2
    exit 1
  fi
  sleep 1
done
[[ -n "$username" ]] || {
  printf 'no successful Pocket ID login was received before timeout.\n' >&2
  exit 1
}
pocket_subject="$(python3 - "$DATABASE_PATH" "$username" <<'PY'
import sqlite3
import sys

database, username = sys.argv[1:]
connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
try:
    row = connection.execute(
        "SELECT subject FROM pocket_sessions WHERE username = ? ORDER BY created_at DESC LIMIT 1",
        (username,),
    ).fetchone()
finally:
    connection.close()
if row is None or not row[0]:
    raise SystemExit("successful Pocket ID session did not contain a subject")
print(row[0])
PY
)"
jq -n \
  --arg clientId "$CLIENT_ID" \
  --arg username "$username" \
  --arg subject "$pocket_subject" \
  --arg testedAt "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{clientId:$clientId, username:$username, subject:$subject, authMethod:"pocket_id_sub", testedAt:$testedAt}' > "$RESULT_MARKER"
chmod 600 "$RESULT_MARKER"
printf 'Pocket ID login test passed for %s.\n' "$username"
