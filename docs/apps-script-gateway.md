# Test Google Sheets gateway

The Android app calls a bound Apps Script API executable. The script owns the
critical section for selecting a prepared row, verifies the workbook schema and
formula template, writes only permitted input cells, and uses a UUIDv7 in
`Transactions!R` for idempotent retries.

This gateway is deliberately pinned to `GOOGLE_SHEETS_TEST_SPREADSHEET_ID`.
Deployment fails closed if the test and production IDs are equal. Production is
not migrated or configured by these commands.

## Deploy and verify

The Google Cloud project must have the Sheets API and Apps Script API enabled.
The Apps Script project and the calling OAuth clients must all use that same
standard Cloud project.

```powershell
npm run gateway:enable-api
npm run gateway:deploy-test
npm run gateway:deploy-test -- --commit
npm run gateway:call
npm run gateway:call -- --function getFinanceSnapshot --set asOf=2026-08-23 --set recentCount=5
```

The deploy command is a dry run unless `--commit` is present. Private script and
deployment IDs are saved under `.google/`, which is ignored by Git.

If Google reports that the caller lacks permission on the first execution,
open the created Apps Script project, select **Project Settings**, then under
**Google Cloud Project** choose **Change project**. Enter the numeric project
number belonging to `.google/credentials.json`, select **Set project**, and run
the commit command again. Google requires this shared standard project for
`scripts.run` callers.

## Household allowlist

Bootstrap initially allows only the deploying owner. `setAllowedUsers` can be
called only by that owner and replaces the complete allowlist. Put an array like
this in a private ignored JSON file and call it with `gateway:call`:

```json
[
  { "email": "owner@example.com", "createdBy": "td_prestosupuesto" },
  { "email": "second@example.com", "createdBy": "second_prestosupuesto" }
]
```

```powershell
npm run gateway:call -- --function setAllowedUsers --json .google/allowed-users.json
```

Both accounts must be able to access the workbook. Keep the allowlist file out
of Git because it contains personal email addresses.

## Android OAuth configuration

Create the following OAuth clients in the same Cloud project:

- an Android client restricted to package `com.prestosupuesto.app` and the APK
  signing certificate SHA-1; and
- a Web application client used as the Google sign-in server client ID.

Copy `.env.app.example` to `.env.local`, add the Web client ID and Apps
Script deployment ID, then generate the native project and build the APK:

```powershell
Copy-Item .env.app.example .env.local
npm run prebuild:android
npm run apk:release:windows
```

The two `EXPO_PUBLIC_` values are public identifiers, not secrets. The native
Android client restriction, Google consent, Apps Script executable access, and
gateway allowlist enforce access.

## Write guarantees

- Only signed-in allowlisted users can call the business functions.
- `LockService.getScriptLock()` serializes both phones.
- Duplicate UUIDv7 requests return the existing transaction.
- Exact `Transactions!A:T` headers, ten expected formulas, and five validations
  are verified before every write.
- D is only replaced for an explicit transaction-currency override; E is only
  replaced for a cross-currency transfer.
- Edits require the original row, date, description, amount, and stable UUID (when present) to still match; otherwise they fail and require a refresh.
- Formula cells are restored from an untouched prepared row before edited input values are written.
- The gateway edits only the selected transaction row; it never edits budgets or balances.
