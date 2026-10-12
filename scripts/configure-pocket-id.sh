#!/usr/bin/env bash
# Register the server-side Pocket ID public client with PKCE and both callbacks.
set -Eeuo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
POCKET_ID_URL="${PRESTO_POCKET_ID_URL:-https://auth.aries.timdumol.com}"
CLIENT_ID="${PRESTO_POCKET_ID_CLIENT_ID:-presto-presupuesto-web}"
API_KEY_FILE="${PRESTO_POCKET_ID_API_KEY_FILE:-$ROOT/.pocket-id-api-key}"
ENV_FILE="${PRESTO_SERVER_ENV_FILE:-$ROOT/.env.server.local}"
LOCAL_CALLBACK="http://localhost:3000/auth/pocket-id/callback"
HOSTED_CALLBACK="https://athena.timdumol.com/auth/pocket-id/callback"

for command in curl jq; do
  command -v "$command" >/dev/null || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 127
  }
done
[[ -r "$API_KEY_FILE" ]] || {
  printf 'Pocket ID API key file is missing or unreadable: %s\n' "$API_KEY_FILE" >&2
  exit 2
}

POCKET_ID_URL="${POCKET_ID_URL%/}"
if [[ "$POCKET_ID_URL" != "https://auth.aries.timdumol.com" ]]; then
  printf 'PRESTO_POCKET_ID_URL must be https://auth.aries.timdumol.com\n' >&2
  exit 2
fi
if [[ ! "$CLIENT_ID" =~ ^[A-Za-z0-9_-]{1,128}$ ]]; then
  printf 'PRESTO_POCKET_ID_CLIENT_ID contains unsupported characters.\n' >&2
  exit 2
fi

umask 077
api_config="$(mktemp /tmp/presto-pocket-id.XXXXXX)"
api_response="$(mktemp /tmp/presto-pocket-id-response.XXXXXX.json)"
api_payload="$(mktemp /tmp/presto-pocket-id-payload.XXXXXX.json)"
api_payload_tmp="$(mktemp /tmp/presto-pocket-id-payload-tmp.XXXXXX.json)"
marker_tmp="$(mktemp "$ROOT/.pocket-id-client-configured.XXXXXX")"
trap 'rm -f "$api_config" "$api_response" "$api_payload" "$api_payload_tmp" "$marker_tmp"' EXIT

api_key_value="$(tr -d '\r\n' < "$API_KEY_FILE")"
[[ -n "$api_key_value" ]] || {
  printf 'Pocket ID API key file is empty: %s\n' "$API_KEY_FILE" >&2
  exit 2
}
printf 'header = "X-API-KEY: %s"\n' "$api_key_value" > "$api_config"
unset api_key_value

api_request() {
  local method="$1" endpoint_path="$2" body_file="${3:-}" http_code
  case "$endpoint_path" in
    /api/oidc/clients)
      ;;
    /api/users)
      ;;
    /api/oidc/clients/*)
      local endpoint_id="${endpoint_path##*/}"
      [[ "$endpoint_id" =~ ^[A-Za-z0-9_-]{1,128}$ ]] || {
        printf 'refusing an unrecognized Pocket ID API path: %s\n' "$endpoint_path" >&2
        exit 2
      }
      ;;
    *)
      printf 'refusing an unrecognized Pocket ID API path: %s\n' "$endpoint_path" >&2
      exit 2
      ;;
  esac
  if [[ -n "$body_file" ]]; then
    http_code="$(curl --silent --show-error --config "$api_config" --output "$api_response" --write-out '%{http_code}' --request "$method" --header 'Content-Type: application/json' --data-binary "@$body_file" "$POCKET_ID_URL$endpoint_path")"
  else
    http_code="$(curl --silent --show-error --config "$api_config" --output "$api_response" --write-out '%{http_code}' --request "$method" "$POCKET_ID_URL$endpoint_path")"
  fi
  if [[ ! "$http_code" =~ ^2[0-9][0-9]$ ]]; then
    printf 'Pocket ID client configuration failed at %s (HTTP %s).\n' "$endpoint_path" "$http_code" >&2
    exit 1
  fi
}

jq -n \
  --arg id "$CLIENT_ID" \
  --arg name "Presto Presupuesto" \
  --arg description "Private household finance analysis workspace" \
  --arg local "$LOCAL_CALLBACK" \
  --arg hosted "$HOSTED_CALLBACK" \
  --arg local_logout "http://localhost:3000/" \
  --arg hosted_logout "https://athena.timdumol.com/" \
  '{
    id: $id,
    name: $name,
    description: $description,
    callbackURLs: [$local, $hosted],
    logoutCallbackURLs: [$local_logout, $hosted_logout],
    isPublic: true,
    pkceEnabled: true,
    requiresReauthentication: false,
    requiresPushedAuthorizationRequests: false,
    skipConsent: false,
    credentials: {},
    launchURL: $hosted_logout,
    isGroupRestricted: false
  }' > "$api_payload"

client_path="/api/oidc/clients/$CLIENT_ID"
client_http="$(curl --silent --show-error --config "$api_config" --output "$api_response" --write-out '%{http_code}' "$POCKET_ID_URL$client_path")"
case "$client_http" in
  404)
    api_request POST '/api/oidc/clients' "$api_payload"
    printf 'Created Pocket ID client: %s\n' "$CLIENT_ID"
    ;;
  2[0-9][0-9])
    jq 'del(.id)' "$api_payload" > "$api_payload_tmp"
    mv "$api_payload_tmp" "$api_payload"
    api_request PUT "$client_path" "$api_payload"
    printf 'Updated Pocket ID client: %s\n' "$CLIENT_ID"
    ;;
  *)
    printf 'Pocket ID client lookup failed (HTTP %s).\n' "$client_http" >&2
    exit 1
    ;;
esac

api_request GET "$client_path"
if ! jq -e \
  --arg id "$CLIENT_ID" \
  --arg local "$LOCAL_CALLBACK" \
  --arg hosted "$HOSTED_CALLBACK" \
  '.id == $id and .isPublic == true and .pkceEnabled == true and (.callbackURLs | index($local) != null) and (.callbackURLs | index($hosted) != null)' \
  "$api_response" >/dev/null; then
  printf 'Pocket ID did not return the expected public PKCE client callbacks.\n' >&2
  exit 1
fi

api_request GET '/api/users'
pocket_subjects="$(jq -er '
  (if type == "array" then . else (.data // .users // []) end)
  | map(select(.username == "tim" or .username == "jopi"))
  | sort_by(.username)
  | if length != 2 or .[0].username != "jopi" or .[1].username != "tim" then
      error("Pocket ID must return exactly the existing tim and jopi users")
    else
      map(.id)
      | if all(.[]; type == "string" and test("^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")) then
          join(",")
        else
          error("Pocket ID user IDs must be UUID subjects")
        end
    end
' "$api_response")"
[[ -r "$ENV_FILE" ]] || {
  printf 'server environment file is missing or unreadable: %s\n' "$ENV_FILE" >&2
  exit 2
}
PRESTO_POCKET_ID_SUBJECTS="$pocket_subjects" python3 - "$ENV_FILE" <<'PY'
import os
import pathlib
import tempfile
import sys

path = pathlib.Path(sys.argv[1])
subjects = os.environ["PRESTO_POCKET_ID_SUBJECTS"]
lines = path.read_text().splitlines()
updated = []
subject_line_written = False
for line in lines:
    key = line.split("=", 1)[0].strip() if "=" in line else ""
    if key == "POCKET_ID_ALLOWED_SUBJECTS":
        if not subject_line_written:
            updated.append(f"POCKET_ID_ALLOWED_SUBJECTS={subjects}")
            subject_line_written = True
    elif key != "POCKET_ID_ALLOWED_USERNAMES":
        updated.append(line)
if not subject_line_written:
    updated.append(f"POCKET_ID_ALLOWED_SUBJECTS={subjects}")

path.parent.mkdir(parents=True, exist_ok=True)
with tempfile.NamedTemporaryFile("w", dir=path.parent, prefix=".pocket-env.", delete=False) as handle:
    temporary = pathlib.Path(handle.name)
    handle.write("\n".join(updated) + "\n")
temporary.chmod(0o600)
os.replace(temporary, path)
PY

jq -n \
  --arg id "$CLIENT_ID" \
  --arg configured_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg subjects "$pocket_subjects" \
  '{clientId:$id, allowedSubjects:($subjects|split(",")), localLoginTestRequired:true, configuredAt:$configured_at}' > "$marker_tmp"
chmod 600 "$marker_tmp"
mv "$marker_tmp" "$ROOT/.pocket-id-client-configured.json"
rm -f "$ROOT/.pocket-id-auth-tested.json"
printf 'Configured Pocket ID subject allowlist for the existing tim and jopi accounts.\n'
printf 'Verified Pocket ID client callbacks for localhost and athena.timdumol.com.\n'
