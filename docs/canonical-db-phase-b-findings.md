# Canonical database: Phase B findings

**Inspected:** 2026-09-28

The initial Phase B run exported the configured **test workbook** through the
Sheets API in read-only mode. A later, separate read-only export and import
dry run against the owner-linked live workbook is recorded below. Neither run
wrote to Google Sheets. Archives and row-level review reports remain under
ignored/private paths; this document contains aggregate counts only.

## Tooling delivered

- `sheets:export-finance` archives Transactions A:T, Accounts A:R, Expenses v3,
  and Exchange Rates with entered, evaluated, formatted, and data-source
  formula values. It checks the transaction/account/budget header contracts,
  records source and canonical timezone metadata, hashes the archive, and writes
  private immutable export files.
- `presto-admin finance import` supports a redacted dry run and an explicit
  transactional commit. It assigns durable UUIDv7 identities for blank source
  IDs, preserves source row fingerprints and formula state, quarantines rows
  without usable dates, and refuses validation issues or a nonempty finance
  schema. A test-workbook commit also requires an explicit database path and
  `--allow-test-workbook`.
- `presto-admin finance verify` compares source row fingerprints and imported
  transaction, valuation, budget, account, quarantine, and rate counts. The
  default output is redacted; a private report is opt-in.
- `presto-admin finance export-sqlite` writes a non-overwriting JSON snapshot
  with table counts and a digest.
- A sanitized fixture generator creates seven synthetic transactions, seven
  categories, six synthetic rate observations, and synthetic account/budget
  rows. The fixture uses test-workbook formula templates but contains no source
  workbook ID, user account names, source-specific category names, or
  transaction descriptions. Generic helper labels and source headers remain.

## Test-workbook dry-run result

The archive importer produced these aggregate classifications. The dated
transaction count is the planning candidate count and includes rows with review
issues; it does not mean those rows are commit-ready.

| Classification | Count |
| --- | ---: |
| Accounts | 42 |
| Categories | 33 |
| Dated transaction candidates | 2,822 |
| Quarantined transaction rows | 23 |
| Budget month snapshots | 564 |
| Nonblank source exchange-rate rows | 83 |
| Normalized source-oriented rate observations | 703 |
| Validation issue entries | 827 |

The 23 quarantined rows correspond to 22 populated rows without a usable date
and one out-of-range date. They remain source-preserved and are not assigned a
made-up effective date.

Validation issue entries by reason were:

| Reason | Entries |
| --- | ---: |
| Invalid amount | 65 |
| Nonpositive ordinary amount | 39 |
| Unexpected destination amount override | 14 |
| Unrecognized operation shape | 31 |
| Unresolved account reference | 9 |
| Unresolved category reference | 639 |
| Unsupported input currency | 30 |

These are issue entries, not distinct transaction counts; a source row can
produce more than one entry. The dry run found no current-period valuation or
budget formula-gap issue. Because the source still has unresolved import
issues, `--commit` was not used. No SQLite rows were changed and the Phase B
migration was not applied to a database.

## Live-workbook export and import dry run

A separate read-only export of the owner-linked live workbook was taken on
2026-09-28. This is a review snapshot, not a frozen cutover export. Its private
archive digest begins `6ea395da`; the private import report and archive remain
under `.google/migration/finance-export-20260928T152448Z-6ea395da/`. The live
export records source timezone `Asia/Manila` and canonical business month
`2026-09`.

The importer dry run reported:

| Classification | Count |
| --- | ---: |
| Accounts | 43 |
| Categories | 33 |
| Dated transaction records in the import plan | 3,038 |
| Quarantined transaction rows | 1 |
| Budget month snapshots | 596 |
| Nonblank source exchange-rate rows | 83 |
| Normalized source-oriented rate observations | 703 |
| Validation issue entries | 829 |

The quarantined transaction is `Transactions` row 2169, whose date evaluates to
2206-03-12, outside the supported import range. It remains source-preserved and
must not be assigned a replacement date.

The 829 validation entries consist of 828 transaction entries affecting 713
source rows and one current-period budget gap. Transaction issue entries by
reason were:

| Reason | Entries |
| --- | ---: |
| Unresolved category reference | 639 |
| Invalid amount (blank source amount) | 66 |
| Nonpositive ordinary amount | 39 |
| Unrecognized operation shape | 32 |
| Unsupported input currency | 29 |
| Unexpected literal destination amount | 14 |
| Unresolved account reference | 9 |

The unresolved category entries span 14 distinct source labels across 537 rows;
the account-reference entries span two labels. The nonpositive amounts comprise
14 negative and 25 zero ordinary entries. These findings require row-level
dispositions; they are not permission to coerce amounts, map names by
similarity, or omit source rows.

There are two current-period findings to resolve before the current month can
feed canonical live totals:

- `Transactions` row 3115, dated 2026-09-01, uses the `Income` helper account
  but does not match a recognized operation shape.
- The real-category `Remaining` cell `Expenses v3!AM12` is blank. Its paired
  current-month `Expense` cell is present; this is a missing value, not a
  calculated formula error.

The current September transaction rows have no J:Q formula gaps or errors. The
export contains 19 historical J:Q error cells across nine pre-September rows;
these remain source provenance and must not be silently replaced with zero or
newly fetched historical FX values. The dry run also found no unresolved
current-period transaction valuation error. Because the live export still has
validation issues, `--commit` was not used. No SQLite rows were changed and the
Phase B migration was not applied.

### Historical account actual-balance records

The owner confirms that the `Accounts` current `Actual Balance` fields are
updated only on the first of each month. In this export they represent values
as of some time on 2026-09-01, not as of the 2026-09-28 export time.
The ledger has September transactions through 2026-09-28, so current book
balances at export time are not directly comparable to those September 1 actual
balances; parity must use a matching as-of boundary.

A separate read-only inspection of `Accounts!W1:BE` found 35 dated balance
columns spanning 2023-09-02 through 2026-09-01, with 40 account rows containing
at least one historical value. The column headers omit 2024-11 and 2026-05.
Among cells with nonblank evaluated values, 1,125 are literals and 58 are
formulas; no cell errors were observed. The W:BE values were inspected in
memory and were not retained in the A:R archive or this report. No account
names or balances are included here.

At the time of this inspection, the Phase B export command archived
`Accounts!A:R` only, so the live import dry run did not include this historical
balance series. The source has date-only transaction effective dates, while
the 2026-09-01 actual was recorded at an unspecified time that day; confirm
same-day ordering before treating it as an August month-end target. Do not
fabricate checkpoints for the two months without headers or interpolate missing
actual balances.

This qualifies the canonical plan's assumption that historical actual balances
are absent: the workbook has dated actual-balance checkpoints for the months
represented in W:BE, while 2024-11 and 2026-05 remain unrecorded there.

For any reviewed historical backfix that changes an account's native balance,
compare corrected ledger balances with the preserved actual-balance checkpoints
and add linked balance adjustments so each checkpoint remains unchanged. Keep
the imported transaction and valuation baseline and every correction revision
auditable. If a historical checkpoint is not independently represented in this
range, retain the source-derived ledger close without asserting that it is a
verified statement balance. Budget correction/reallocation remains a separate
category-level operation.

### Phase B checkpoint follow-up (2026-09-30)

The exporter now archives `Accounts!W:BE` alongside A:R. The importer stores
each named account/date cell in `finance_account_balance_checkpoints`, retaining
the date-header cell, entered/evaluated/formatted source cell, original row and
column, and formula/literal/blank/error state. Verification compares those
values exactly, and a finance commit now requires this checkpoint range. The
2026-09-28 live review archive predates this addition and must be re-exported
before any future import; no live workbook was re-read or changed for this
tooling update.

The checked-in sanitized fixture now has six synthetic accounts, 35 synthetic
date headers, and 210 source cells. On 2026-09-30 the new migration and importer
were exercised against a disposable SQLite database. The import committed with
zero validation issues; verification reported source-row-exact parity,
including all 210 historical checkpoint cells; SQLite export included the new
table. This round trip used only the sanitized fixture and did not apply the
schema to a production database.

### Fresh live review export (2026-09-30)

A new read-only review export was taken after checkpoint support was added. It
is stored privately under
`.google/migration/finance-export-20260930T174425Z-044f9e47/`; its digest
starts `044f9e47`. The archive contains 43 Accounts records and the W:BE
checkpoint range. The import planner expands the named accounts to 1,505
checkpoint cells (43 accounts × 35 dates), including blank trailing rows.

The redacted dry run classified 3,039 dated transaction candidates, one
quarantined transaction row, 596 budget snapshots, 83 raw rate rows, and 703
normalized rate observations. It still reports 829 validation entries: 828
transaction entries across 713 source rows and one current-period budget gap.
The transaction issue counts remain 66 invalid amounts, 39 nonpositive
ordinary amounts, 14 unexpected destination-amount literals, 32 unrecognized
operation shapes, 9 unresolved account references, 639 unresolved category
references, and 29 unsupported currencies. The unresolved references still
cover 14 category labels and two account labels; no name mappings were guessed.

The September transaction issue remains `Transactions` row 3115
(`unrecognized_operation_shape`). The current budget pair still has a blank
real-category Remaining cell at `Expenses v3!AM12`. No current-period J:Q
valuation gap or error was reported. The out-of-range row 2169 remains the one
quarantined transaction. This is a review snapshot, not a frozen cutover
export; it wrote no Sheets cells and the dry run wrote no SQLite rows.

### Fresh live review export after source correction (2026-09-30)

A new read-only export was taken after the owner corrected the source formula.
It is stored under the ignored private path
`.google/migration/finance-export-20260930T215347Z-db5a23ae/`; its digest begins
`db5a23ae`. The current-period budget pair now has an evaluated value and the
dry run reports no budget formula gaps or errors.

The import rules now apply case-insensitive account/category/currency matching
and use a digest-bound private owner-review file for the reviewed exceptions.
The source row JSON and fingerprint remain unchanged when an approved mapping
is applied. Opening rows for real accounts are stored separately from ordinary
transactions, while virtual opening helpers and blank reconciliation sinks are
retained in quarantine.

| Classification | Count |
| --- | ---: |
| Accounts | 43 |
| Categories | 33 |
| Dated ordinary transactions | 3,118 |
| Real-account opening balances | 31 |
| Quarantined or explicitly ignored source rows | 73 |
| Populated transaction input rows classified | 3,222 |
| Budget month snapshots | 596 |
| Nonblank source exchange-rate rows | 83 |
| Normalized source-oriented rate observations | 703 |
| Validation issues | 0 |
| Opening sign/type review warnings | 1 |

The 73 quarantined rows include blank-amount reconciliation sinks, four
zero-valued virtual opening helpers, owner-reviewed exclusions, and one
invalid-date row. The owner marked the SB Platinum account invalid for opening
balance use. Its seed is quarantined; the account remains inactive and its 16
imported historical transactions remain linked for audit. One negative opening
seed is assigned to an account with a blank or different type and remains
flagged for owner review. No source signs were changed.

The dry run and commit were followed by a read-only verifier against a
disposable SQLite database. It reported source-row-exact parity for all 3,118
ordinary transactions, their 6,236 valuation records and 24,944 J:Q source
cells, 31 opening balances, 1,505 historical account checkpoint cells, all 596
budget snapshots, and 703 normalized rates. It also confirmed every populated
transaction input row was imported, quarantined, or explicitly ignored. The
source verifier compares imported source cells and provenance; it does not yet
recompute account totals, valuations, or budget behavior. Those calculations
remain part of Phase C. The disposable database is under `/tmp`; no production
database or workbook was changed.

### Owner review follow-up (2026-10-01)

The owner explicitly reviewed the remaining negative seed as an opening-balance
row. The digest-bound review file now records `Transactions` row 8 as
`opening_balance`; the importer continues to preserve the source row, fingerprint,
and blank source account type. A fresh dry run against the same archive reports
zero validation issues and zero opening-balance warnings. This review changed
no SQLite rows and wrote nothing to Sheets.

## Remaining Phase B gates

The provisional migration stores money and rates as decimal text, preserves
transaction revision snapshots, versioned valuation rows and per-cell J:Q
provenance, snapshots budget values and exact reallocation impacts, retains raw
rate rows, and maps recognized Date/Close blocks into source-oriented
pair/date/rate observations. The importer does not infer reciprocal rates. The
live source scan confirms canonical decimal text and exact arithmetic; it does
not set a global imported scale. The owner has selected ECB as the future rate
provider. The opening-balance warning has since been cleared by the follow-up
owner review above.

The checkpoint archive and sanitized-fixture SQLite round trip are complete.
The current-period budget gap and transaction review issues are resolved for
the current review export, and its populated rows have explicit
import/quarantine/ignore classifications. Complete calculated
account/valuation/budget comparisons in Phase C. Before production cutover, use
a final frozen export of the owner-linked workbook and repeat source-row
classification and verification.
