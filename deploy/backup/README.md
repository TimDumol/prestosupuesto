# Presto offsite backups

Installed 2026-10-03. Two live backup/pull cycles and full restored parity for
all 29 finance tables passed. Google Drive was activated on 2026-10-03; all
encrypted packs passed verification, and a restore directly from Drive passed
checksums, SQLite integrity and exact row parity for all 29 finance tables.
Scope is the Presto finance service, not all services on Athena.

Athena runs `/usr/local/sbin/presto-backup` every six hours at 00:15, 06:15,
12:15 and 18:15 UTC, with up to five minutes jitter and missed-run catch-up.
It uses SQLite's online backup API on `/opt/presto-finance/data/presto.sqlite`,
checks integrity, and runs the deployed admin image against that immutable copy
in a separate container with no network. Backups contain the full SQLite database,
all current finance tables in the portable JSON format, production `.env`, Compose
configuration, image version and SHA-256 checksums. Source snapshots and cutover
recovery artifacts have a permanent tag excluded from ordinary retention.

The encrypted server repository lives in `/var/lib/presto-backup/repository`.
The recovery password is mode 0600 at `/etc/presto-backup/password` on Athena
and `.backup/password` on this laptop. Save it in your password manager separately
from both machines. It is essential for recovery. Never commit `.backup`.

Laptop jobs live in `~/.config/systemd/user/presto-backup-laptop*`. The daily pull
copies the encrypted server repository under its backup lock, validates it, then
imports snapshots into `.backup/laptop-repository`. This is an independent
repository: server deletions do not remove laptop snapshots. The user session
must be running; daily jobs catch up at next login/startup. Logs are available
through `journalctl --user -u presto-backup-laptop.service`.

Both repositories retain every snapshot within seven days of the newest snapshot,
30 daily, 12 weekly and 12 monthly snapshots. Retention excludes permanent
snapshots. Server storage is pruned weekly; laptop storage is pruned after the
monthly restore drill. No old deployment backups are deleted by these jobs.

Monthly verification reads every encrypted data pack. A laptop user timer also
runs `scripts/backup/drive-drill.sh` on the second day of each month, restoring
directly from Drive and checking exact finance parity. The laptop additionally
extracts the newest finance backup into temporary private storage and checks
file checksums, SQLite integrity, every finance table count and exact JSON row
contents. Temporary recovered plaintext is deleted on exit.

Freshness checks run on this laptop every six hours and show a critical desktop
notification if the laptop backup is older than 36 hours, or an enabled server /
Drive backup is older than 12 hours. Notifications require an active desktop
notification service; this laptop currently has none running. The check falls back
to logging the failure and marking its systemd unit failed. This is not an
independent always-on monitoring service.
Failures also appear in systemd service status and the journal.

## Google Drive connection

Drive is active in the [presto-backup folder](https://drive.google.com/drive/folders/1YRsGsEESO4tSTHYYs70rnn0Kg2ADWomD).
The rclone authorization listener uses the `drive.file` scope and the project's
existing desktop OAuth client. The shared rclone client failed with Google API
quota errors, so it is not used. After sign-in:

```bash
scripts/backup/finish-drive.sh
```

This installs a private rclone config on Athena, initializes the dedicated
`rclone:presto-drive:presto-backup` repository, enables copying to it after each
backup, and immediately runs a backup. restic encrypts contents and metadata;
rclone is the transport, so a second crypt layer is unnecessary. The account
must have sufficient free storage. The Google Drive API is enabled in the existing OAuth project.
Server Drive credentials can delete cloud
backups; independently retained laptop snapshots provide additional protection.

If authorization expires, run:

```bash
python3 scripts/backup/authorize-drive.py
```

Open the localhost URL in that private log on this laptop. Do not share the log:
it contains OAuth tokens after successful authorization.

## Manual verification and recovery

```bash
scripts/backup/laptop.sh
scripts/backup/laptop-drill.sh
RESTIC_PASSWORD_FILE="$PWD/.backup/password" .backup/bin/restic -r .backup/laptop-repository snapshots
```

For recovery, choose a snapshot by ID. Extract without restoring root ownership:

```bash
mkdir -m 700 /tmp/presto-recovery
RESTIC_PASSWORD_FILE="$PWD/.backup/password" .backup/bin/restic --no-cache \
  -r .backup/laptop-repository dump SNAPSHOT_ID /var/lib/presto-backup/staging \
  > /tmp/presto-recovery/snapshot.tar
tar --no-same-owner -xf /tmp/presto-recovery/snapshot.tar -C /tmp/presto-recovery
python3 scripts/backup/verify-snapshot.py /tmp/presto-recovery/var/lib/presto-backup/staging
```

On a replacement host, rebuild the recorded application version from the repository
or use the retained image if available. Configure Pocket ID access (its separate
service is outside this backup's scope). Stop finance before installing the restored
SQLite database; remove obsolete WAL/SHM files, set owner 10001:10001, install the
saved `.env` with mode 0600, then start finance and verify health and finance reads
before accepting writes. Keep the database restore API disabled. The JSON export
can alternatively be restored with `presto-admin finance restore-sqlite` and its
SHA-256 digest into a new empty database. Full SQLite is the primary recovery path.

The automation does not change production ledger rows or authentication gates.
