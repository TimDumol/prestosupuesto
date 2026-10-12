# Android client guide

The Kotlin/Compose application in `native-android/` uses the canonical Rust API.
It has Entry, Activity, Budget, Accounts and Settings destinations. The server
owns accounting, currency valuation, budget calculation, revisions and access.

## Entry

Expenses, salary/extra income/refunds, transfers and budget moves use exact
BigDecimal amounts encoded as decimal strings. Localized separators and bounded
arithmetic are supported; final amounts must have at most two decimal places.
Quick entry accepts `currency amount / account / description`, resolves account
aliases and suggests categories. An unmatched account requires manual selection.

Every user change marks the persisted draft as touched, including account,
category, date and note selections. Refreshes preserve touched drafts, and replacing
one requires confirmation. Save and add another clears entry content while
retaining the posting date and selections. Month-end balancing suggests successive
donors and posts on the chosen month-end.

## Authentication and storage

Pocket ID sign-in opens `/auth/pocket-id/mobile?client=native-android` and returns
to `prestosupuesto-native://pocket-id/callback`. Strict callback validation and a
one-time handoff precede the bearer-session exchange. Tokens are encrypted using
Android Keystore. Room stores confirmed snapshots, drafts and an outbox scoped
to the authenticated owner and API origin.

Save atomically persists the write, its optimistic overlay and the next draft.
Stable UUIDv7 identifiers prevent duplicates after a lost acknowledgment.
Foreground and WorkManager dispatchers serialize writes. Permanent failures stay
visible in Settings for review; revision conflicts never silently overwrite
another writer. Cached data and drafts remain available offline.

## Recovery

Verified full-server snapshots are encrypted with AES-GCM in app-private storage.
The client validates digest, schema, table counts and decryptability before
publishing a generation, retains eight generations and falls back from damaged
copies. Successful backups have a six-hour interval; failed attempts have a
fifteen-minute throttle. Restore requires confirmation, no pending local writes,
an empty server and administrator-enabled recovery mode.

## Validation

`npm run native:test` runs JVM tests, Android lint and APK builds.
`npm run native:test -- --device --serial=SERIAL` adds synthetic device tests.
Device tests use fixtures and MockWebServer, with no household finance writes.
Checks cover parsing, entry workflows, persisted drafts, atomic Room operations,
retry deduplication, revision conflicts, recovery and chart interactions.
