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
python3 - "$BASE" <<'PY'
import json, sys
from pathlib import Path
base=Path(sys.argv[1])
text=(base/'drive-authorization.log').read_text()
decoder=json.JSONDecoder()
token=None
for i,c in enumerate(text):
    if c!='{': continue
    try: candidate,_=decoder.raw_decode(text[i:])
    except ValueError: continue
    if isinstance(candidate,dict) and 'access_token' in candidate:
        token=candidate
        break
if token is None:
    raise SystemExit('Drive browser authorization has not completed.')
client=json.loads((base/'drive-client.json').read_text())
(base/'rclone.conf').write_text('[presto-drive]\ntype = drive\nscope = drive.file\nclient_id = '+client['client_id']+'\nclient_secret = '+client['client_secret']+'\ntoken = '+json.dumps(token)+'\n')
(base/'rclone.conf').chmod(0o600)
PY
KEY=$PRESTO_SSH_KEY
scp -i "$KEY" "$BASE/rclone.conf" "$PRESTO_SSH_USER@$PRESTO_SSH_HOST":/tmp/presto-backup-rclone.conf
ssh -i "$KEY" "$PRESTO_SSH_USER@$PRESTO_SSH_HOST" 'sudo -n install -m 600 /tmp/presto-backup-rclone.conf /etc/presto-backup/rclone.conf; rm -f /tmp/presto-backup-rclone.conf; sudo -n env RESTIC_PASSWORD_FILE=/etc/presto-backup/password RCLONE_CONFIG=/etc/presto-backup/rclone.conf restic -r rclone:presto-drive:presto-backup snapshots >/dev/null 2>&1 || sudo -n env RESTIC_PASSWORD_FILE=/etc/presto-backup/password RCLONE_CONFIG=/etc/presto-backup/rclone.conf restic -r rclone:presto-drive:presto-backup init'
ssh -i "$KEY" "$PRESTO_SSH_USER@$PRESTO_SSH_HOST" 'sudo -n sh -c "umask 077; printf %s rclone:presto-drive:presto-backup > /etc/presto-backup/drive-repository"; sudo -n systemctl start presto-backup.service'
