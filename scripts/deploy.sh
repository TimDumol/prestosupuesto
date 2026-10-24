#!/usr/bin/env bash
# Test the auth prerequisite, build locally, stream the image to Lima, and deploy with Ansible.
set -Eeuo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
# Optional private deployment settings, excluded from Git.
if [[ -r "$ROOT/.env.deploy.local" ]]; then
  # foxguard: ignore[bash/source]
  . "$ROOT/.env.deploy.local"
fi
: "${PRESTO_SSH_HOST:?Set PRESTO_SSH_HOST in .env.deploy.local or the environment}"
: "${PRESTO_SSH_USER:?Set PRESTO_SSH_USER in .env.deploy.local or the environment}"
: "${PRESTO_SSH_KEY:?Set PRESTO_SSH_KEY in .env.deploy.local or the environment}"
ANSIBLE_DIR="$ROOT/deploy/ansible"
ENV_FILE="${PRESTO_SERVER_ENV_FILE:-$ROOT/.env.server.local}"
CONFIG_MARKER="$ROOT/.pocket-id-client-configured.json"
AUTH_MARKER="$ROOT/.pocket-id-auth-tested.json"
DOMAIN='athena.timdumol.com'
PUBLIC_URL="https://$DOMAIN"
LOG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/presto-deploy.XXXXXX")"
trap 'rm -rf "$LOG_DIR"' EXIT
chmod 700 "$LOG_DIR"

run_quietly() {
  local label="$1" log_name="$2" log
  shift 2
  case "$log_name" in
    bootstrap|web-deps|web-check|docker-build|image-transfer|deploy|health)
      ;;
    *)
      printf 'unrecognized release log name: %s\n' "$log_name" >&2
      exit 2
      ;;
  esac
  log="$LOG_DIR/$log_name.log"
  printf '%-18s' "$label"
  if "$@" >"$log" 2>&1; then
    echo 'ok'
  else
    local status=$?
    echo 'FAILED'
    tail -n 80 "$log" >&2
    exit "$status"
  fi
}

for command in node npm docker curl gzip python3 ssh ansible-playbook jq getent; do
  command -v "$command" >/dev/null || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 127
  }
done
node -e 'const [major, minor, patch] = process.versions.node.split(".").map(Number); if (major !== 24 || minor < 22 || (minor === 22 && patch < 1)) { console.error("Deployment requires Node.js 24.22.1 or newer in the 24.x series (CVE-2026-111497)."); process.exit(1); }'
[[ -r "$ENV_FILE" ]] || {
  printf 'server environment file is missing: %s\n' "$ENV_FILE" >&2
  exit 2
}
[[ -f "$CONFIG_MARKER" ]] || {
  printf 'run scripts/configure-pocket-id.sh before deployment.\n' >&2
  exit 2
}
[[ -f "$AUTH_MARKER" ]] || {
  printf 'run scripts/test-pocket-id-auth.sh and complete a real localhost login before deployment.\n' >&2
  exit 2
}

set -a
# This local environment file is ignored by Git and must stay private.
# foxguard: ignore[bash/source]
. "$ENV_FILE"
set +a
CLIENT_ID="${POCKET_ID_CLIENT_ID:-presto-presupuesto-web}"
jq -e --arg id "$CLIENT_ID" --arg subjects "$POCKET_ID_ALLOWED_SUBJECTS" \
  '.clientId == $id and (.allowedSubjects | sort) == ($subjects | split(",") | sort)' \
  "$CONFIG_MARKER" >/dev/null || {
  printf 'configured Pocket ID client does not match the server environment.\n' >&2
  exit 2
}
jq -e --arg id "$CLIENT_ID" --arg subjects "$POCKET_ID_ALLOWED_SUBJECTS" \
  '.clientId == $id and .authMethod == "pocket_id_sub" and (.username == "tim" or .username == "jopi") and (.subject as $subject | ($subjects | split(",") | index($subject)) != null)' \
  "$AUTH_MARKER" >/dev/null || {
  printf 'a successful tim or jopi Pocket ID login is required before deployment.\n' >&2
  exit 2
}
if ! getent ahostsv4 "$DOMAIN" >/dev/null; then
  printf 'DNS for %s is not resolving from this machine.\n' "$DOMAIN" >&2
  exit 2
fi

production_env="$LOG_DIR/presto-finance.env"
ansible_vars="$LOG_DIR/ansible-vars.json"
python3 - "$ENV_FILE" "$production_env" "$ansible_vars" <<'PY'
import json
import os
import pathlib
import re
import sys

source, destination, vars_path = map(pathlib.Path, sys.argv[1:])
values = {}
for raw in source.read_text().splitlines():
    line = raw.strip()
    if not line or line.startswith("#") or "=" not in line:
        continue
    key, value = line.split("=", 1)
    key = key.strip()
    if not re.fullmatch(r"[A-Z][A-Z0-9_]*", key):
        continue
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
        value = value[1:-1]
    values[key] = value

required = (
    "GOOGLE_CLIENT_ID",
    "GOOGLE_CLIENT_SECRET",
    "APPS_SCRIPT_DEPLOYMENT_ID",
    "ALLOWED_EMAILS",
    "TOKEN_ENCRYPTION_KEY",
    "POCKET_ID_ISSUER",
    "POCKET_ID_CLIENT_ID",
    "POCKET_ID_ALLOWED_SUBJECTS",
)
if any(not values.get(key) for key in required):
    raise SystemExit("The local server environment is missing required finance or Pocket ID settings.")
if values["POCKET_ID_ISSUER"].rstrip("/") != "https://auth.aries.timdumol.com":
    raise SystemExit("POCKET_ID_ISSUER must be https://auth.aries.timdumol.com.")
import uuid
subjects = [subject.strip() for subject in values["POCKET_ID_ALLOWED_SUBJECTS"].split(",") if subject.strip()]
if len(subjects) != 2 or len(set(subjects)) != 2:
    raise SystemExit("POCKET_ID_ALLOWED_SUBJECTS must contain exactly two unique Pocket ID subjects.")
try:
    for subject in subjects:
        uuid.UUID(subject)
except ValueError:
    raise SystemExit("POCKET_ID_ALLOWED_SUBJECTS must contain Pocket ID subject UUIDs.")

values.update({
    "PUBLIC_ORIGIN": "https://athena.timdumol.com",
    "GOOGLE_REDIRECT_URI": "https://athena.timdumol.com/auth/google/callback",
    "POCKET_ID_REDIRECT_URI": "https://athena.timdumol.com/auth/pocket-id/callback",
    "COOKIE_SECURE": "true",
})
fcm_file = os.environ.get("FCM_SERVICE_ACCOUNT_FILE") or values.get("FCM_SERVICE_ACCOUNT_FILE")
extra_vars = {"presto_env_file": str(destination)}
if fcm_file:
    fcm_path = pathlib.Path(fcm_file).expanduser().resolve()
    account = json.loads(fcm_path.read_text())
    if account.get("type") != "service_account" or not account.get("private_key"):
        raise SystemExit("Invalid FCM service-account file.")
    extra_vars["presto_fcm_service_account_file"] = str(fcm_path)
    values["FCM_SERVICE_ACCOUNT_FILE"] = "/app/secrets/fcm.json"
destination.write_text("".join(f"{key}={value}\n" for key, value in values.items()))
destination.chmod(0o600)
vars_path.write_text(json.dumps(extra_vars, separators=(",", ":")))
vars_path.chmod(0o600)
PY

if [[ "${PUBLIC_ORIGIN:-}" != 'http://localhost:3000' ]]; then
  printf 'local environment must keep PUBLIC_ORIGIN=http://localhost:3000; deploy.sh sets Athena values in a private temporary file.\n' >&2
  exit 2
fi

SSH_HOST="$PRESTO_SSH_HOST"
SSH_USER="$PRESTO_SSH_USER"
SSH_KEY="$PRESTO_SSH_KEY"
SSH_DEST="$SSH_USER@$SSH_HOST"
SSH_ARGS=(-o BatchMode=yes -o ConnectTimeout=15 -i "$SSH_KEY")
IMAGE_NAME='presto-finance'
IMAGE_TAG="${PRESTO_IMAGE_TAG:-release-$(date -u +%Y%m%d%H%M%S)}"
IMAGE_REF="$IMAGE_NAME:$IMAGE_TAG"
DOCKER_PLATFORM="${PRESTO_DOCKER_PLATFORM:-linux/amd64}"

echo "Presto finance release -> $PUBLIC_URL (Pocket ID login was verified locally)"
run_quietly 'bootstrap' bootstrap \
  bash -c 'cd "$1" && shift && exec ansible-playbook site.yml --tags bootstrap' _ "$ANSIBLE_DIR"
run_quietly 'web dependencies' web-deps npm ci --prefix "$ROOT/web"
run_quietly 'web typecheck' web-check npm run --prefix "$ROOT" web:typecheck
run_quietly 'docker build' docker-build \
  docker build --pull --platform "$DOCKER_PLATFORM" --tag "$IMAGE_REF" "$ROOT"

stream_image() {
  docker save "$IMAGE_REF" \
    | gzip -1 \
    | ssh "${SSH_ARGS[@]}" "$SSH_DEST" 'gzip -d | sudo -n docker load'
}
run_quietly 'image transfer' image-transfer stream_image

run_quietly 'deploy' deploy \
  bash -c 'cd "$1" && shift && exec ansible-playbook site.yml --extra-vars "presto_image=$1" --extra-vars "@$2"' \
  _ "$ANSIBLE_DIR" "$IMAGE_REF" "$ansible_vars"
run_quietly 'health check' health \
  curl --fail --silent --show-error --retry 12 --retry-delay 5 --retry-all-errors --max-time 15 "$PUBLIC_URL/healthz"
printf 'Deployed %s\n' "$PUBLIC_URL"
