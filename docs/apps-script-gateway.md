# Test Google Sheets gateway

The Android app calls a bound Apps Script API executable. The script owns the
critical section for selecting a prepared row, verifies the workbook schema and
formula template, writes only permitted input cells, and uses a UUIDv7 in
`Transactions!R` for idempotent retries.

The test gateway is pinned to `GOOGLE_SHEETS_TEST_SPREADSHEET_ID`. Deployment
fails closed if the test and production IDs are equal.

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
# The app invokes reconcileAccounts from its Accounts screen.
```

The deploy command is a dry run unless `--commit` is present. Private script and
deployment IDs are saved under `.google/`, which is ignored by Git.

If Google reports that the caller lacks permission on the first execution,
open the created Apps Script project, select **Project Settings**, then under
**Google Cloud Project** choose **Change project**. Enter the numeric project
number belonging to `.google/credentials.json`, select **Set project**, and run
the commit command again. Google requires this shared standard project for
`scripts.run` callers.

## Redeploy the live gateway

`gateway:deploy-live` uploads the current `apps-script/Code.js`, creates an
immutable code version, updates the existing live API deployment in place, and
checks that the live snapshot returns historical budget months. The dry run is
read-only; add `--commit` to publish the version:

```powershell
npm run gateway:deploy-live
npm run gateway:deploy-live -- --commit
```

Updating the existing deployment keeps its deployment ID stable for app builds
that already use it. The live script project is stored separately from the test
gateway in `.google/apps-script-live-deployment.json`.

## Finance maintenance gate

The live gateway has a persistent owner-controlled gate for its three finance
write functions. Each writer checks the gate while holding the same Apps Script
lock used for the write. Enabling it waits for any current write to finish,
then blocks later calls from both current and older app builds. Reads continue
to work. This gate does not cover a separate Apps Script deployment or manual
workbook edits; inventory those deployments and restrict manual edits as part
of cutover.

After deploying the gateway version that includes the gate, the owner can
inspect or change it through the live deployment. In PowerShell:

```powershell
$env:GOOGLE_APPS_SCRIPT_DEPLOYMENT = '.google/apps-script-live-deployment.json'
$env:GOOGLE_APPS_SCRIPT_TOKEN = '.google/apps-script-live-redeploy-token.json'
npm run gateway:call -- --function getMaintenanceGate
npm run gateway:call -- --function setMaintenanceGate --set enabled=true
npm run gateway:call -- --function getMaintenanceGate
```

Pass `--set enabled=false` to reopen finance writes. Enabling is a cutover
action; verify the returned gate state before proceeding with database cutover.

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

## Archive access

The native Android app signs in through Pocket ID and writes to the canonical
Rust API. This gateway supports archive maintenance tooling only. Keep the live
workbook maintenance gate closed while SQLite is canonical.

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
- Reconciliation writes only the manual `Actual Balance` account cells plus the matching month-end reconciliation rows; it never writes calculated balances or budgets.
- Reconciliation retries reuse UUIDs and update an existing same-date/account reconciliation row instead of creating a duplicate.
- The owner can pause the three finance write functions with the persistent
  maintenance gate; it is checked under their shared write lock. This does not
  disable other deployments or manual workbook editing.
