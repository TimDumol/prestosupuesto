# Presto Native Android

Kotlin/Compose Android client for the canonical finance API. Package:
`com.prestosupuesto.nativeapp`.

## Build and install

Use Java 17 and Android SDK 36. Gradle downloads the pinned dependencies.

```bash
scripts/build-and-install-native-android.sh
scripts/build-and-install-native-android.sh --install
# Multiple devices:
scripts/build-and-install-native-android.sh --install --serial=SERIAL
```

The script runs JVM tests and Android lint and copies the signed development APK
to `PrestoNative.apk` at the repository root. The default signing certificate is
Android's local debug certificate; this is a device trial build, not a Play Store
release. Configure production signing for store distribution.

The public API defaults to `https://athena.timdumol.com`. Override it with
`./gradlew -PfinanceApiUrl=https://your-server.example :app:assembleDebug`.
Local HTTP is allowed only in debug builds. Tokens are acquired interactively
and stored encrypted using Android Keystore; no credentials belong in Gradle
properties or source files.

## Authentication

The client opens `/auth/pocket-id/mobile?client=native-android&code_challenge=…`
and accepts only `prestosupuesto-native://pocket-id/callback`. The server must
support this callback. The app saves a fresh S256 verifier for each login and
supplies it when exchanging the 90-second, single-use handoff. Intercepting the
custom-scheme callback alone cannot obtain a bearer session. The server separately
uses PKCE with Pocket ID and binds its callback to the initiating browser.
Tokens are encrypted with Android Keystore. Sessions have a fixed 30-day lifetime;
expiry clears credentials while keeping offline drafts and queued entries. A
failed logout reports the revocation failure and retains credentials for retry.

Server migration 0015 requires the updated Android client for new mobile logins.
Existing bearer sessions remain valid until logout or expiry. This auth-only
migration leaves finance tables unchanged; the updated recovery reader accepts
schema 15. Older schema-14 recovery archives still require a matching server for
restoration. Presto sessions are independent of the Pocket ID browser session;
signing out of Pocket ID alone does not revoke a Presto session.

## Features

- Five destinations: Entry, Recent, Budget (overview/history), Accounts and Settings.
- Expenses, salary/extra income/refunds, transfers with destination currency
  amounts, reallocations and multi-donor month-end balancing.
- Save and add another retains the selected posting date.
- Before a new entry is queued, same-day duplicate warnings compare amounts within
  three cents, currency, directional account/category IDs and income treatment.
  Transfers also compare destination currency and amount. Description similarity
  ranks matches but is optional. Cached and pending offline entries are included;
  uncached partner entries require a refresh. Keep editing preserves the draft;
  Save anyway confirms only the reviewed draft. Edits and queued retries are excluded.
- Fuzzy description autocomplete across available history fills budget and account
  together. Compact amount entry supports arithmetic with help shown on focus.
  Fuzzy pickers, date picking and durable create/edit drafts.
- Cached activity search, exact account/category filtering, month/type filters,
  daily groups, currency totals, revision-guarded edits and historical corrections.
- Delete eligible synced entries from transaction details with confirmation. Deletion
  requires sign-in and a connection, reverses balances and budgets, and preserves
  the operation ID across retries after a lost response. Resolve pending writes first.
- Budget rollover, cash coverage, expected-income completion and interactive
  stacked history with selection, category isolation and reallocation toggle.
- Read-only account balances; reconciliation and administration stay on the web.
- Save shows immediate progress, commits to the local outbox off the UI thread, then
  opens Activity or resets for another entry without waiting for server sync.
  Accepted saves run in the repository's lifetime scope, survive cancellation of
  the calling screen, and share a nonblocking admission gate with deletion.
  Save diagnostics record elapsed time and the remaining pending count;
  `PrestoSave` logcat events also identify received taps and completed navigation callbacks.
- Room outbox, optimistic pending entries, stable UUIDv7 retry IDs, serialized
  foreground/WorkManager sync and a pending notification. Permanent server
  failures appear in Activity, transaction details and Settings with retry/review actions.
  Settings includes Show logs and Copy logs for the latest 200 in-memory diagnostic
  events, capped at 2,000 message characters each. Logs reset with the app process;
  they are never written to disk, and Logcat output is limited to debug builds.
  Fast successful syncs with no pending writes are silent; failures, writes and syncs
  taking at least two seconds remain visible. Credentials and request bodies are excluded.
  Review a failed operation as a draft against
  confirmed revisions; it remains queued until its replacement commits locally.
  Revision conflicts never silently overwrite another writer's changes.
- Automatic verified AES-GCM recovery archives: streamed download/validation,
  bounded encryption chunks, eight generations, six-hour successful backup
  interval and fifteen-minute failed-attempt throttle. Restore requires an empty
  server with administrator enabled recovery mode and no local pending writes.
- Isolated synthetic demo without signing in. Demo writes never enter the outbox.

Financial amounts use `BigDecimal` and decimal strings on the API. Money entry
rejects more than two final decimal places, matching the existing input hint;
there is no silent rounding. The server remains the accounting authority.
Optional notes are saved on a new line in the transaction description; the
combined text is limited to 500 characters. Editing structured transactions
preserves their recorded values without reinterpreting descriptions.

## Partner transaction notifications

The everyday Android package supports direct Firebase Cloud Messaging alerts.
Place its public client configuration at `app/google-services.json` before building.
In Settings, enable **Partner transactions** and allow Android notification permission
on each phone. Alerts include the actor, amount, currency and first line of the
transaction description. Tapping fetches the current transaction details, including
older/backdated entries outside the recent cache. Edits, deletes, imports and
reconciliation do not generate alerts.

Device registrations belong to the authenticated bearer session. Logout removes
registrations; expired sessions are excluded. The phone also checks the recipient
and session hash before showing data-only pushes. Stable event IDs suppress retries
across process restarts. Offline entries generate alerts only after the canonical
server commits them. The separate `deviceTest` package disables Firebase entirely;
CI builds without the client config still compile with partner alerts disabled.

On the server, set `FCM_SERVICE_ACCOUNT_FILE` to the private service-account JSON.
The account needs `roles/firebasecloudmessaging.admin` on the client Firebase project.
`scripts/deploy.sh` copies this secret separately and mounts it read-only; it never
enters the image or APK. Validate authorization without notifying a device with:

```bash
node scripts/check-fcm-auth.mjs /secure/path/presto-fcm.json
```

The single-server worker drains a durable SQLite outbox every five seconds, retries
transient failures with backoff, and expires alerts after 24 hours or 12 attempts.
Notifications are best effort; a force-stopped app, notification settings or network
conditions can delay/prevent delivery. Push tokens and queued alerts are operational
metadata excluded from finance recovery exports. Migration 0016 changes the server
schema; the updated Android recovery reader accepts schemas through 16.

## UI previews

The native UI uses warm neutral surfaces, teal accents, and shared
status, search, and icon components. Entry has a fixed Save action; activity
shows compact two-line transaction rows directly below search, with month/type
filters in a sheet and no aggregate summary block. Tapping a row opens details
before editing, including explanations for read-only and pending entries. Budget category bars show spending against monthly funding
plus rollover and budget moves.

Emulator captures using synthetic demo data: [Entry](visual/entry.png),
[Budget](visual/budget.png), [Activity](visual/activity.png), and
[Accounts](visual/accounts.png), and [Autocomplete](visual/autocomplete.png).

## Validation

```bash
cd native-android
./gradlew :core:test :app:lintDebug :app:assembleDebug :app:assembleDeviceTestAndroidTest
./gradlew :app:connectedDeviceTestAndroidTest
# From the repository root, select a device explicitly:
# npm run native:test -- --device --serial=SERIAL
```

Device tests install the separate `com.prestosupuesto.nativeapp.devicetest`
package ("Presto Test"), with its own storage and login callback scheme. Gradle's
test cleanup can remove that package without replacing or removing the everyday
`com.prestosupuesto.nativeapp` app. Use `connectedDeviceTestAndroidTest` or the
`native:test -- --device` script; instrumented tests target only this isolated variant.

Device tests use synthetic data and a loopback MockWebServer. They exercise Room
atomicity/reopen/owner isolation, lost-acknowledgment deduplication, retained
conflicts, encrypted recovery corruption/rotation, chart interaction, entry and
draft recreation. They do not log into Pocket ID or write household finance data.
The Android test asset is a server export of the repository's sanitized fixture.
JVM tests include fixed regression vectors for money parsing.

The implementation uses manual constructor injection, direct OkHttp and a simple
saved destination state instead of Hilt, Retrofit and Navigation 3. Five root
screens and one shared repository do not need those additional frameworks yet.
Room schemas are versioned; add explicit migrations for future schema changes.

## Client architecture

See [the client guide](../docs/native-android-client-plan.md) for storage,
synchronization, recovery and validation behavior. Confirmed finance data loads
from the canonical service after sign-in; each installation creates its own
verified encrypted recovery generations.
