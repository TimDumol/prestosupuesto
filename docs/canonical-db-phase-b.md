# Canonical database: Phase B tools

Phase B introduces a private, read-only Sheets archive and an explicit SQLite
admin command. Google Sheets remains untouched by these commands.

## Export a workbook

Use the test workbook for discovery and sanitized fixture work:

```sh
npm run sheets:export-finance -- --test
```

The command reads `GOOGLE_SHEETS_TEST_SPREADSHEET_ID`. To archive another
workbook, pass `--spreadsheet` explicitly. It checks the transaction A:T
contract, account metadata headers, and dated budget month pairs before writing
`.google/migration/finance-export-*/`. The output contains evaluated values,
user-entered literals/formulas, data-source formulas, and formatted values for
Transactions A:T, Accounts A:R and W:BE, and the Expenses v3 and Exchange Rates
grid ranges. The W:BE `accountCheckpoints` section aligns with original Accounts
row numbers and preserves each date header and cell's entered, evaluated,
formatted, and formula state. It records source timezone separately from the
Europe/Madrid business timezone.
The manifest includes workbook identity/settings, per-sheet header fingerprints,
source-row counts, export digest, and formula-state counts.

Export files are private financial data. Keep the complete directory outside
Git and preserve it unchanged once selected as an import source. The importer
creates a private `identity-map.json` beside it. That file binds generated
UUIDv7 IDs to source rows and reference names and must travel with the archive
and off-host backup.

## Import and compare

The admin CLI opens the same `DATABASE_PATH` by default as the service. Pass a
separate SQLite path for isolated dry runs:

```sh
npm run db:finance-import -- --input .google/migration/finance-export-... --database /tmp/presto-finance-shadow.sqlite
npm run db:finance-import -- --input .google/migration/finance-export-... --database /path/to/presto.sqlite --commit
npm run db:finance-verify -- --input .google/migration/finance-export-... --database /path/to/presto.sqlite
```

Committing any test-workbook archive requires both an explicit shadow database
path and `--allow-test-workbook`; it cannot silently use the service's default
database path.

Without `--commit`, import validates the archive and reports redacted counts;
it changes no SQLite rows. Commit runs all SQLx migrations and inserts the
reference data, dated transactions and imported revision snapshots, two
side-specific valuation baselines per transaction/revision, J:Q cell
provenance, budget snapshots and exact reallocation
impact, account actual balances and dated historical checkpoints, raw rate
source rows, normalized source-oriented rate observations, and quarantined
invalid or undated transaction rows in one database transaction. It refuses to merge
into populated finance tables and treats a repeated source digest as an
idempotent success. Existing auth tables are preserved.

The default verifier reports counts and parity status without displaying
financial values or transaction IDs. Pass `--report PATH` only when row-level
diagnostics are needed; that private report includes source row numbers.

Create the checked-in synthetic fixture from a private test-workbook export:

```sh
npm run db:finance-fixture -- .google/migration/finance-export-...
```

The generator checks the archive digest, requires a test-workbook export, and
writes fake accounts, categories, descriptions, amounts, dates, rates, and
historical account checkpoints under
`fixtures/canonical-db/`. It copies only transaction formula templates and
generic source headers needed to exercise the importer. The fixture includes
expense, income, cross-currency transfer, reallocation, signed/zero
reconciliation, a historical valuation gap/error, and synthetic checkpoint
values covering literal, formula, blank, and error cell states.

## Export SQLite

Create a private JSON interchange file from the database. The export reads all
finance tables from one consistent SQLite read snapshot, includes table counts,
the schema version and latest finance-operation timestamp, refuses to overwrite
an existing output, and prints the file digest without finance values:

```sh
npm run db:finance-export -- --database /path/to/presto.sqlite --output /secure/path/finance-snapshot.json
```

Restore that export into a fresh database path by supplying the digest printed
by the export command:

```sh
npm run db:finance-restore -- --input /secure/path/finance-snapshot.json \
  --sha256 64_CHARACTER_EXPORT_DIGEST --database /secure/path/restored.sqlite
```

Restore refuses an existing target, applies current migrations, checks the
export's table set, columns, counts, digest, SQLite integrity, and foreign-key
references, and creates private database files. Use a new disposable target
for recovery drills; this is a local SQLite snapshot interchange format, not
the planned off-host Sheets backup.

The interchange snapshot includes category-only correction records and their
operation receipts so a restore can reconstruct current open-month category
values and the correction flag on the original transaction.

## Phase B boundaries

Money and rates are stored as decimal text; the SQLite schema does not impose a
global amount scale. The importer uses exact decimal-string arithmetic for
reallocation-impact aggregation. The Phase C calculation model uses exact
rational FX arithmetic and the owner-selected half-to-even posting rule.
Imported Exchange Rates rows keep their raw formulas, evaluated
values, and source row fingerprints. The Phase B importer also maps recognized
repeated Date/Close blocks into normalized observations while preserving source
pair direction; this does not select a future provider or infer reciprocal
rates. The unknown pre-history opening balance remains unresolved.

The test workbook is for formula and FX fixtures. The current test export dry
run was rejected for review issues and wrote no database rows. Production
finance imports must use one final frozen workbook export selected by the owner
after the source freeze and digest checks in the canonical DB plan.
