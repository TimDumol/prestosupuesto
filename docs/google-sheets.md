# Local Google Sheets test tools

These commands let us inspect and modify the real workbook before wiring the
Android UI to it. They use Google's official Node.js client and the Sheets v4
API. No spreadsheet ID, OAuth secret, refresh token, or service-account key is
committed to Git.

## 1. Configure Google Cloud

1. Create or select a project in Google Cloud Console.
2. Enable the **Google Sheets API**.
3. Configure the Google Auth consent screen. For a personal Gmail account,
   choose an external audience and add your own address as a test user.
4. Create an OAuth client with application type **Desktop app**.
5. Download its JSON file to `.google/credentials.json` in this repository.

The first local command opens a browser for consent and saves a refresh token
to `.google/token.json`. Both files are ignored by Git. The CLI requests the
`spreadsheets` scope because append and patch tests need write access. Google
applies that scope to the spreadsheet file, not to an individual tab.

For unattended local/CI tests, put a service-account key at
`.google/service-account.json`, set `GOOGLE_APPLICATION_CREDENTIALS`, and share
the spreadsheet with the key's `client_email`. OAuth is simpler for the first
personal test.

## 2. Configure this repository

In PowerShell:

```powershell
Copy-Item .env.sheets.example .env.sheets.local
New-Item -ItemType Directory .google
```

Edit `.env.sheets.local` and paste either the full Google Sheets URL or its ID.
Set the transaction and budget tab names exactly as they appear in the workbook.

Authenticate and verify access:

```powershell
npm run sheets:auth
```

If you later change OAuth scopes, delete `.google/token.json` and authenticate
again so Google can issue a token with the new scope.

## 3. Read and inspect

List the transaction header columns:

```powershell
npm run sheets:inspect
```

Inspect the budget tab instead:

```powershell
npm run sheets:inspect -- --sheet "Expenses v3"
```

Create a private structural report of the existing workbook:

```powershell
npm run sheets:structure
```

The report is saved to `.google/workbook-structure.json`, which is ignored by
Git because it can contain personal financial data. By default it samples the
first 250 rows and 52 columns of every grid tab and records formulas, calculated
and displayed values, number formats, validations, notes, named ranges, protected
ranges, merged cells, filters, tables, charts, locale, timezone, and recalculation
settings. Increase or target the inspection when needed:

```powershell
npm run sheets:structure -- --rows 2000 --columns 80
npm run sheets:structure -- --range "Transactions!A1:AZ500" --range "Rates!A1:M5000"
```

Summarize ledger occupancy, formula coverage, and transaction shapes without
printing descriptions or amounts:

```powershell
npm run sheets:probe-ledger
```

Write one formula-preserving transaction only to the configured test copy. This
command refuses production, validates the A:T header fingerprint when metadata
is installed (or the original A:Q fingerprint before migration) and dropdown
sources, requires a prepared row with all ten formulas and five validation rules,
and writes only the permitted raw-input and metadata cells:

```powershell
npm run sheets:test-transaction -- --kind expense --date 2026-08-23 --amount 0.01 --from-category "Your category" --from-account "Your account"
```

Review the dry run, then repeat it with `--commit`. The command re-reads the row,
verifies that every formula is byte-for-byte unchanged, and records a private
audit line under `.google/test-writes.jsonl`.

Read any A1 range as a table or JSON:

```powershell
npm run sheets:read -- --range "Transactions!A1:J20"
npm run sheets:read -- --range "'Expenses v3'!A1:F" --json
```

Fetch transaction and budget tabs together, which approximates an app startup
sync and avoids two API round trips:

```powershell
npm run sheets:snapshot > snapshot.json
```

Build the app-facing finance model from the real formula results. This resolves
the active monthly budget pair, account balances, currencies, and recent ledger
rows into `.google/finance-snapshot.json`:

```powershell
npm run sheets:finance-snapshot -- --as-of 2026-08-23
```

Install the approved hidden test metadata and account-activity flag. The command
is test-only and dry-run by default:

```powershell
npm run sheets:migrate-test-schema
npm run sheets:migrate-test-schema -- --commit
```

This creates hidden `Transactions!R:T` metadata and `Accounts!R` `IsActive`.
It does not backfill UUIDs into historical rows and never targets production.

Deploy the serialized test gateway after completing the Cloud setup described
in [apps-script-gateway.md](apps-script-gateway.md):

```powershell
npm run gateway:deploy-test
npm run gateway:deploy-test -- --commit
npm run gateway:call
```

## 4. Generic append for other sheets

This command is useful while exploring ordinary tabular sheets. It deliberately
refuses the discovered `Transactions` ledger because that ledger has prepared
per-row formulas; use `sheets:test-transaction` for the test copy instead.

Input object keys are matched to the real header row without depending on column
order. Matching ignores case, spaces, underscores, and punctuation, but unknown
fields are rejected. First copy and edit the example so its keys match your
actual headers:

```powershell
Copy-Item scripts/sheets/examples/transaction.example.json transaction.local.json
npm run sheets:append -- --data transaction.local.json
```

That is a dry run. It shows the destination columns without writing. Commit the
same row only after reviewing it. The command also scans for formula-bearing
columns and refuses to overwrite formula cells unless explicitly overridden:

```powershell
npm run sheets:append -- --data transaction.local.json --commit
```

For a quick test without a JSON file, repeat `--set`:

```powershell
npm run sheets:append -- --set "Date=2026-08-23" --set "Description=API test"
```

## 5. Generic patch for other sheets

This command also refuses the discovered `Transactions` ledger until it has a
stable ID column. It can be used on other ordinary tabular sheets.

Use a stable unique column such as `Transaction ID`; do not locate mutable rows
by row number. The command requires exactly one match and only writes the cells
included in the patch, preserving formulas and other columns.

```powershell
npm run sheets:patch -- --key-column "Transaction ID" --key "txn-123" --set "Description=Updated test"
```

Again, this is a dry run. Add a stale-value check and `--commit` to write only if
the sheet contains the expected previous value when the command reads it:

```powershell
npm run sheets:patch -- --key-column "Transaction ID" --key "txn-123" --set "Description=Updated test" --expect "Description=API test" --commit
```

## Using Sheets as the primary backing store

This is feasible for a personal-finance app if writes are append-mostly and every
transaction has a stable UUID. The implemented Apps Script gateway holds a
script-wide lock across row selection, validation, writing, and result reading,
so the two phones cannot select the same prepared row. Direct CLI writes remain
test utilities and are not the mobile integration path.
Keep input tabs free of per-row formulas where possible; compute derived views in
separate tabs with array formulas. The mobile app should also maintain an encrypted
local cache and an outbox so entries survive poor connectivity.

Never embed a service-account private key or desktop OAuth client secret in the
APK. The recommended bridge keeps workbook credentials server-side and gives
the app a narrow transaction API.

Sheets is less suitable if the app becomes multi-user, needs strict transactions
across several tabs, or grows to high write volume. The CLI intentionally gives
us a way to test your real schema and latency before committing to that design.

The existing workbook's discovered contract, formula ownership, and all five
verified transaction mappings are documented in
[workbook-contract.md](workbook-contract.md).
