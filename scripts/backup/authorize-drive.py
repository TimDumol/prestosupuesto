#!/usr/bin/env python3
"""Authorize a dedicated Drive connection using the project's existing desktop client."""
import json
import os
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[2]
base = root / '.backup'
base.mkdir(mode=0o700, exist_ok=True)
client = json.loads((root / '.google/credentials.json').read_text())['installed']
private = base / 'drive-client.json'
private.write_text(json.dumps({'client_id': client['client_id'], 'client_secret': client['client_secret']}))
private.chmod(0o600)
environment = os.environ.copy()
environment.update(RCLONE_DRIVE_SCOPE='drive.file', RCLONE_DRIVE_CLIENT_ID=client['client_id'], RCLONE_DRIVE_CLIENT_SECRET=client['client_secret'])
log = base / 'drive-authorization.log'
with log.open('w') as output:
    log.chmod(0o600)
    subprocess.run([str(base / 'bin/rclone'), 'authorize', 'drive', '--auth-no-open-browser'], env=environment, stdout=output, stderr=subprocess.STDOUT, check=True)
