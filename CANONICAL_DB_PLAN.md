# Canonical finance database plan

**Status:** Phases B and C are complete. Phase D's Pocket ID-authenticated
server routes and native/browser client adapters are implemented; all database
features remain disabled by default in Rust. Production now selects canonical DB
and enables its normal finance gates; restore mode remains disabled.
Phase C's DB read model matches the digest-bound owner-linked review snapshot by
value, all archived J:Q formulas replay without mismatches, and eight sanitized
formula-reference scenarios match the DB valuation model across sixteen
valuation sides. Owner decisions for money handling, FX source, historical
budget closes, rollback window, and backup priorities are recorded below.
A fresh live export confirms the repaired budget formula and source precision
review is complete. Digest-bound owner-review import and source-row verification
passed against a disposable database; one opening-balance type warning was
subsequently owner-reviewed as an opening-balance row, and a fresh dry run
reports zero validation issues or review warnings. Disposable transaction,
reconciliation, month-close, and local SQLite export/restore drills now pass.
A physical mobile snapshot and recovery drill passed on the one available Android phone.
A second-phone check is follow-up, not a cutover gate. Production cutover began
on 2026-10-01: the live Apps Script deployment is version 8 and its finance
maintenance gate is enabled. The missing-ID importer bug is fixed. Fresh frozen
exports after the October budget formula repair match; their digest-bound
production import and source-row/read-model parity passed with zero mismatches.
Future November budget templates are excluded from canonical import at the
owner's direction. A consistent pre-import SQLite backup is preserved, old
Apps Script versions 1–5 are retired, and both remaining execution entries are
fenced. Canonical server/browser/Android deployment is live. Owner-confirmed browser and
phone Pocket ID checks and the phone’s verified production snapshot passed; DB
writes are enabled with Sheets still frozen. The first real phone entry committed with one new ledger row and one operation
receipt. Owner confirmed browser propagation and the refreshed encrypted phone
snapshot. The canonical backend is live; see
the private cutover handoff (kept outside public version control). The authenticated
snapshot, encrypted native-device retention, and gated restore path are
implemented; Drive export remains an optional follow-up.

**Household timezone:** Europe/Madrid. Effective finance dates are calendar
dates and should be stored without timezone shifts. Keep UTC instants for audit
timestamps. The inspected test-workbook snapshot reported Asia/Manila; retain
recorded source timezone metadata separately, not as the canonical business
timezone.

## 1. Recommendation and scope

Make the existing Rust/Axum service's SQLite database the single shared,
canonical finance store for Android and the browser workspace. Add finance
tables through SQLx migrations in the existing `DATABASE_PATH` database. The
deployed database already lives on the persistent `/app/data` volume; continue
using consistent local SQLite backups. Phone snapshots are the primary
additional recovery point; the optional off-host finance copy is described in
section 10.

Keep Google Sheets as a frozen archive and an explicit rollback destination
during a defined transition period. After cutover, normal client reads and
writes go through the Rust service. Sheets must not remain a second writable
finance database. A separate backup workbook receives scheduled, one-way
DB-to-Sheets exports; it is never a client write target while SQLite is canonical.

The canonical database stays on the server. Room caches, drafts and queued writes
support offline entry; verified encrypted snapshots on each phone provide device
recovery points. Optional off-host archive exports do not block finance entry.

This plan also does not switch to Actual Budget. Its current documentation says
multi-currency is not supported natively. The documented workaround is
experimental and does not preserve this app's native-account balances,
date-based EUR valuations, or transfers with different sent and received
amounts. See [Actual's multi-currency documentation](https://actualbudget.org/docs/budgeting/multi-currency/).

## 2. Current system and migration boundary

The current implementation has three data paths that must converge:

| Area | Current behavior | Target behavior |
| --- | --- | --- |
| Hosted browser | Rust API authenticates users, then proxies finance requests to Apps Script. | Rust API reads and writes SQLite directly. |
| Android | The app signs in to Google and calls Apps Script directly. It caches finance data and stores pending writes locally. | The app calls the Rust API; its cache/outbox retries the same idempotent operation against that API. |
| Google Sheets | `Transactions`, `Accounts`, `Expenses v3`, and `Exchange Rates` contain inputs and formula-derived results. | Frozen source archive plus an optional best-effort backup workbook; clients write only to Rust. |

Relevant code and contracts:

- Server startup, SQLite pool, WAL, and SQLx migrations: [`server/src/main.rs`](server/src/main.rs), [`server/migrations`](server/migrations).
- Browser routes currently proxy to Apps Script: [`server/src/server.rs`](server/src/server.rs).
- Android API and sign-in: [`Api.kt`](native-android/app/src/main/java/com/presto/nativeapp/Api.kt).
- Android durable cache and write queue: [`Repository.kt`](native-android/app/src/main/java/com/presto/nativeapp/Repository.kt), [`Sync.kt`](native-android/app/src/main/java/com/presto/nativeapp/Sync.kt).
- Sheet row layout, formula ownership, currency behavior, and reconciliation: [`docs/workbook-contract.md`](docs/workbook-contract.md), [`docs/sheet-understanding-review.md`](docs/sheet-understanding-review.md).
- Apps Script gateway and existing Sheets CLI helpers: [`apps-script/Code.js`](apps-script/Code.js), [`scripts/sheets`](scripts/sheets).

Both household users currently share one workbook. Keep the first database
model as one household ledger with `created_by` audit metadata; do not partition
the ledger by login email. A future multi-household product would need an
explicit tenant model and is outside this migration.

## 3. Behaviors the database must preserve

The initial database-backed release must retain these established semantics:

1. **Transaction flows:** expense, income, account transfer, budget
   reallocation, and reconciliation. A reconciliation can have a negative or
   zero adjustment even though ordinary app entries must be positive.
2. **Budgeting:** budgets are reported in EUR. Monthly base amounts and
   rollover produce available balances; month-end balancing can move money
   from one or more surplus categories into a deficit category. Historical
   views can show reallocation effects or ignore them. The app performs the
   existing consistency check that total allocated budget does not exceed actual
   funds across accounts; it does not depend on the workbook's Accounts R8 cell.
3. **Accounts:** real and active account pickers, inactive accounts retained
   for history, native balances, actual statement balances, and reconciliation
   differences.
4. **Currency:** EUR, PHP, and USD; transaction date determines historical
   conversion; account summaries use a current/as-of conversion; the rare
   source-currency override remains explicit; cross-currency transfers retain
   the actual received destination amount.
5. **Reliability:** UUIDv7 request IDs, retry deduplication, stale-edit checks,
   a durable mobile outbox, and atomic reconciliation/budget-balancing batches.
6. **Read models:** recent transactions, current budgets, all available
   historical budget months, account balances, and active/inactive metadata.

The sheet remains the reference behavior until the database implementation
passes value-by-value comparisons. In particular, do not replace historical
formula results with newly calculated rates or guessed opening balances.

## 4. Proposed database model

Names below are conceptual. Finance amounts and rates use canonical decimal
text in SQLite, with exact base-10 arithmetic in Rust.

### 4.1 Reference data

- **`finance_accounts`**: stable ID, original/display name, account type,
  currency, `is_real`, `is_active`, and audit timestamps. Include system/helper
  accounts needed to represent ledger flows, but expose only real active
  accounts in ordinary pickers.
- **`finance_account_balance_checkpoints`**: dated actual-balance cells from
  the source workbook, retaining each W:BE date header and formula/literal/
  blank/error state. Keep the recorded date separate from any inferred
  statement-period meaning.
- **`finance_categories`**: stable ID, original/display name, and real/helper
  status. Preserve sheet order if the UI depends on it.
- **`category_budget_periods`**: category, monthly base amount, and effective
  start month. Record each post-cutover budget change here; it applies from that
  month forward and must not rewrite already closed balances.
- Use stable IDs internally. Preserve original names and names from the sheet
  export so imports, reports, and fallback writes can map exactly. No category
  or account administration is added in this phase.

### 4.2 Canonical transaction ledger

Use a `finance_transactions` table with at least:

- UUIDv7 ID, operation kind, effective date, description, and creation audit
  fields;
- source amount and input currency;
- nullable destination amount and destination currency;
- source/destination category and account references;
- flags recording whether the sheet's `Currency` and `To Amount` cells were
  literals or formulas;
- per-cell source state for J:Q (formula, literal, blank, or error), distinct
  from the semantic D/E override flags;
- revision/version used for guarded edits;
- source workbook, sheet, row, and original-row fingerprint for imported rows.

Represent reconciliation as a first-class ledger kind, with explicit rules
allowing its signed/zero amount while continuing to reject zero/negative normal
entries. The current app's month-end budget balancing writes one reallocation
entry per donor/destination pair; keep those entries individually addressable
and group an app submission into an idempotent batch.

Use a unique constraint on transaction ID. Every mutation, including an edit,
has a separate UUIDv7 operation ID; a multi-row submission has one batch operation
ID. Persist the authenticated actor, normalized payload fingerprint, affected
transaction IDs, and response in `finance_operations`, in the same SQL transaction
as all ledger, valuation, reconciliation, and revision changes. Authorize first,
then return a recorded result for an identical retry before checking its now-stale
expected revision. Reusing an operation ID with different content is an error.
Transaction uniqueness alone does not deduplicate successive edits.

Keep operation records for the supported offline/retry lifetime, including the
rollback window; initially retain them without automatic expiry. Commit guarded
updates and reconciliation read/modify/write steps under one serialized database
write transaction. Preserve prior transaction revisions and valuations for audit
and restore, with the operation ID linking every change.

### 4.3 Conversion values and exchange rates

Add a `transaction_valuations` table keyed by transaction and side (`from` or
`to`). It should preserve the effective values represented by sheet columns
J:Q, including:

- source/destination account currency;
- exchange multiplier to EUR and any multiplier to the source account's native
  currency;
- calculated native amount and EUR amount;
- the effective date, source, and FX policy/rate-set version used.

This is intentional historical data, not a disposable cache. For imported
transactions, store the formula-evaluated values as the migration baseline.
For new transactions and edits to accounting inputs, calculate valuations using
the database FX policy and persist the applied rates with the transaction revision.
Description-only edits preserve valuations. Retain original imported valuations
and revision history so a later FX-policy change cannot erase the baseline.

Add an `exchange_rates` table for dated currency-pair rates and separately
represent current/as-of rates used to display account totals in EUR. Normalize
rate direction in the database (for example, EUR per one unit of currency),
but retain the original sheet pair/direction and source metadata on import.
For imported sheet rates, do not assume reciprocal rates or derive a direct pair
unless the sheet's formula contract proves that is equivalent. For new ECB
rates, retain the published currency-per-EUR observations and derive conversion
pairs from those EUR-based observations using decimal arithmetic.

**FX cutover policy:** Freeze the source workbook's pre-cutover historical rate
observations as literal imported values, with their original dates, pair
directions, and provenance. Keep each transaction's evaluated pre-cutover
valuation as its historical baseline; never replace it with a newly fetched
historical quote. After cutover, fetch the ECB's daily euro foreign-exchange
reference rates directly from its Data Portal into `exchange_rates`. Preserve
the published quote, which is units of foreign currency per EUR, along with the
source date, retrieval time, pair, and rate-set/policy version. Normalize rates
for calculations without discarding the published observation. Use the exact
or most recent prior stored observation for a transaction date; weekends and
holidays therefore use the preceding available business-day rate. Keep actual
received amounts as entered values for transfers. The request path must not
depend on a live API call. If the newest stored ECB observation is more than
three TARGET operating days behind the current date, allow the transaction but
flag its EUR valuation as stale. Count TARGET operating days, excluding
weekends, New Year's Day, Good Friday, Easter Monday, May 1, Christmas Day, and
December 26. The ECB says its reference rates are usually published each working
day except TARGET closing days, and in principle on all TARGET operating days
([publication page](https://www.ecb.europa.eu/stats/policy_and_exchange_rates/euro_reference_exchange_rates/html/index.eu.html),
[rate framework](https://www.ecb.europa.eu/stats/pdf/exchange/Frameworkfortheeuroforeignexchangereferencerates.et.pdf),
[TARGET operating days](https://www.ecb.europa.eu/paym/pdf/target/current/targetoverview.pdf)).
Record the applied observation date. Sync DB rate
observations to the separate backup workbook as literal values. Define how to
handle later corrections to an already-used ECB observation by retaining the
new observation as a version and automatically refreshing valuations in open
months that used it. Preserve the previous valuation in correction history.
Do not change valuations or budget closes for reconciled months. The original
source workbook remains frozen as the pre-cutover archive.

### 4.4 Budget and reconciliation history

- **`budget_month_snapshots`**: imported per-month/per-category expense,
  remaining balance, reallocation impact, and source-reported base when known.
  Preserve all available history exactly as reported. April 2025 is the first
  retained budget snapshot; pre-April carry-in is unknown and must not be
  inferred. A month remains open to correction until the explicit `Close month`
  action is recorded after end-of-month reconciliation. Account reconciliation
  alone does not close the budget month. Closing captures an immutable snapshot
  and fixes its reported close. Do not copy the cutover-time base into earlier
  months unless that historical value is independently verified.
- **`budget_month_closures`**: one immutable closure event per month, linked to
  its idempotent `finance_operations` record. Store the actor and timestamp via
  the operation/event metadata, plus a digest-bound snapshot of all category
  closing values. The close action and snapshot commit atomically. Imported
  historical snapshots retain their original source baseline; do not invent
  historical closure timestamps or operation records.
- **`finance_category_corrections` / `finance_category_correction_items`**:
  append-only, operation-linked corrections for transactions in closed months.
  Keep the original transaction and its source month unchanged, store the
  reason and signed category expense deltas, and apply those deltas atomically
  to the current open month's category snapshots. Positive expense deltas lower
  remaining balance; negative deltas raise it. These operations never affect
  account balances, and the original transaction is marked as corrected in the
  read model. Closing the current month includes the applied category values.
- **`budget_opening_state`** (or equivalent): verified category closing
  balances immediately before the first day of the cutover month in
  Europe/Madrid. The cutover month adds its base once and includes all transactions
  effective in that month, including imported ones. Preserve any exceptional
  cutover-month formula effects as explicit, verified adjustments.
- **`reconciliation_batches` / `reconciliation_items`**: selected month-end,
  account, actual native balance, calculated variance, actor, and linked
  transaction ID. Keep the latest actual balance available for the account
  screen while retaining each past reconciliation. `finance_accounts.ledger_revision`
  is the optimistic concurrency token, incremented by every live ledger change;
  each submission carries the revision read with the account balance.
- **`account_opening_state`**: treat the explicit May 1, 2025 opening-balance
  rows as opening seeds for their real accounts, preserving the signed values.
  Negative credit-card seeds must remain at or below zero. The owner-reviewed
  SB Platinum seed is quarantined because that account is invalid for opening
  balance use. Verify parity before cutover and never count an opening seed
  again as a normal transaction.

#### Historical changes and rollover

Retain historical editing and backdated entry with a separate correction audit
record. While a month remains open, backdated entries and corrections may
recalculate that month's current budget results. After end-of-month
reconciliation, the explicit `Close month` action fixes the month's reported
close. After a month is closed, a backdated correction must not change that
month's close or any later closed-month budget balances. Do not propagate
correction deltas from a closed month through later closed snapshots. Preserve
each reported close as the value reconciled at that time. A correction recorded
in a closed month has no account-balance effect; record its category deltas as
an audited adjustment in the current open month, and flag the original
transaction. Use each month's base as it was effective then, and never apply
today's base to every historical month.

The workbook's literal remaining cells are deliberate frozen month results: they
prevent later changes to a category's monthly base from retroactively changing
closed balances. Import these values as exact monthly closing snapshots, not as
unexplained resets or additive adjustments. Unless another source verifies old
base amounts, mark historical monthly bases unknown; the base visible at
cutover applies from its recorded effective month and must not be copied backward.
For future months, persist the calculated close as a snapshot and version the
base by effective month. Backdated changes to an open month may update its
current results. Backdated changes after reconciliation may update transaction
detail and correction history, but must preserve that month's and later budget
balances.
Preserve the formula text and evaluated result for nonstandard cells as source
provenance. The digest-bound 2026-09-30 owner-linked archive contains 15 J:Q
formula cells with off-row or broken references across 9 rows (14 off-row
references and one `#REF`). An independent formula replay matched all 30,326
calculated outputs and reproduced 19 source formula errors as errors. Keep the
saved formula results as imported baselines; review the affected dates before
enabling edits to those transactions. Exact imported snapshots can still be
shown without classifying every historical literal.

#### Reconciliation policy

For migration parity, retain the existing gateway's behavior explicitly: use the
current all-ledger native balance at submission, with the selected month-end as the
adjustment's effective date. This is not a historical statement-balance calculation;
the UI must describe the basis. An as-of-month-end calculation would be a separate
behavior change requiring an agreed rule and new parity expectations.

Keep one adjustment transaction per account/effective date. A new reconciliation
operation replaces that adjustment after adding its previous signed amount back
to the current balance; retain all submission versions in reconciliation history.
Require an expected account-ledger revision, incremented by every mutation affecting
that account, to reject a submission based on a concurrently changed balance.
An identical operation retry returns its saved result without recomputation or
rewriting actual balances. The latest successfully committed reconciliation
submission supplies the account screen's actual balance, even when its effective
date is older; store submission time and effective date separately. Historical
actual balances absent from the source remain unknown, never inferred as facts.

### 4.5 Exact money representation

Do not use SQLite `REAL` as the source of truth for money or rates. The live
export contains source inputs at varying displayed scales, including values
with more than two decimal places; formula-derived budget results also use
additional decimal places. There is no single safe integer scale for every
imported amount, rate, and valuation. Preserve imported numeric values at their
source precision, keep the raw cell payload and formula state, and do not round
intermediate calculations. For new posted transactions, validate the currency's
posting precision; round only at that posting boundary. Display money to two
decimal places. SQLite `REAL` is never the source of truth.

## 5. Backfill: Sheets to SQLite

Implement this as a repeatable, auditable pipeline, not a one-off manual copy.

### 5.1 Extract

Add a Sheets export command, for example `sheets:export-finance`, using the
existing Google Sheets API setup. It writes a private manifest and NDJSON/JSON
files under an ignored path such as `.google/migration/`. Export:

- `Transactions!A:T`: input values, evaluated values, formulas, metadata, and
  original row numbers;
- `Accounts!A:R`: headers, active/real flags, currencies, current actual
  balances, and evaluated native/EUR summaries;
- `Accounts!W:BE`: dated historical balance checkpoints with entered,
  evaluated, formatted, and formula state preserved per cell;
- `Expenses v3`: category metadata, all historical month headers and paired
  expense/remaining cells, and formula results;
- `Exchange Rates`: used range, headers, dates, pair direction, formula and
  evaluated values, plus workbook locale/timezone and schema fingerprints.

The export should not print transaction descriptions, balances, rates, email
addresses, or other financial values to the terminal. It should report counts,
row numbers, and redacted error reasons. Validate exact known headers and
formula/value ranges before producing a committed export.

### 5.2 Import

Add a Rust/SQLx admin command, for example `presto-admin finance import`, that
uses the same migrations and database path as the server. Its default is dry
run; commit requires an explicit flag. The command should:

1. Validate manifest schema, source workbook identity and recorded source
   timezone separately from the canonical Europe/Madrid business timezone, and
   transaction/account/category fingerprints.
2. Import reference data, FX rows, transactions, transaction valuation
   snapshots, budget history, account actual balances and dated historical
   account checkpoints, and reconciliation state in one SQL transaction.
3. Preserve existing sheet transaction IDs. For historical rows with blank IDs,
   allocate UUIDv7 IDs once in a durable manifest mapping the frozen export's
   workbook/tab/row to ID and original fingerprint. Reusing that export reuses
   the mapping; do not derive identity from mutable content. Keep historical `Created At`/`Created By` null
   when the sheet has no value; record import time separately.
4. Within a frozen export, treat source workbook + tab + original row as an
   idempotent import key. If a row
   with that key has changed since a previous import, stop for review rather
   than silently inserting a second transaction.
5. Reject duplicate IDs, unsupported currencies, unresolved account/category
   names, invalid operation shapes, and unresolved valuation errors in the open
   current period after the latest reconciliation boundary. Historical
   pre-boundary formulas/errors may be imported with their source state intact;
   use the verified reconciliation/opening balance and closed budget snapshots
   as the active baseline. Produce a redacted error report; do not drop rows.
   Preserve undated or invalid-date source rows in a quarantined import review
   set instead of inventing a date or silently omitting them. Review and record
   formula gaps per legacy cell; they are not automatically defects or user
   overrides.
6. Record an import-run ID, source digest, counts, generated IDs, and source
   watermark in `finance_import_runs`.

Owner-reviewed source decisions are kept in a private file bound to the exact
export digest. Matching of account/category names and currency codes ignores
case. For the reviewed 2026-09-30 archive, preserve the source row while mapping
row 950's category to Groceries, use the source Amount and disregard the
destination amount on row 775, ignore row 1878, and classify row 2943's
reimbursement as income. Blank reconciliation sink rows and zero-valued virtual
opening helpers are quarantined. Explicit real-account May 1, 2025 opening rows
are stored as account state rather than ledger transactions. These dispositions
are not transferable to a changed export without review.

The owner marked the SB Platinum account invalid for opening-balance use. Keep
the source account inactive for historical references, quarantine its opening
seed, and preserve its linked transaction history.

Shadow imports use disposable databases and never establish production identity.
The production import is a fresh full import from the final frozen export into
empty finance tables in the existing auth database. Abort if production finance
tables already contain unrelated data. Retrying the same manifest is idempotent;
do not merge a newer export into an earlier import. A changed export requires an
explicit reset of unactivated finance data and a new manifest. Preserve auth tables.
This avoids treating row positions as stable identities across live edits, row
movement, or deletions. Persist the final identity manifest off-host with backups.

### 5.3 Verify the backfill

Provide a read-only compare command that creates a redacted parity report. It
must compare:

- row counts and source-row coverage by transaction kind;
- all imported transaction inputs and formula/literal flags;
- formula state (formula, literal, blank, or error) for every imported J:Q
  valuation cell, kept separate from semantic D/E override flags; preserve
  historical states as provenance and record review outcomes for current-period
  cells;
- transaction source/destination currencies, native amounts, and EUR amounts;
- current native and EUR account balances and actual balances;
- every dated historical account checkpoint, including formula/literal/blank/
  error source state and original date headers;
- every imported category/month: base, expense, remaining, and reallocation
  effect;
- all rate dates/pairs needed by imported transactions.

Every source row must be classified as imported, quarantined, or explicitly
rejected with a reason. No cutover if there are unexplained differences, missing
source rows, or ambiguous duplicate matches.

## 6. Database-backed service and client cutover

Keep public JSON shapes stable where possible so UI changes are limited. Move
the business logic behind these server operations to SQL-backed services:

- finance snapshot read (transactions, budgets/history, accounts);
- transaction create and guarded edit;
- batch reconciliation, including actual native balance and linked
  adjustment transactions;
- batch budget balancing/reallocation;
- reference data for active accounts/categories and historical budget months.

All writes must enforce the same rules currently enforced in Apps Script:
UUIDv7 idempotency, operation mappings, valid account/category references,
cross-currency transfer amount rules, stale revision checks, and audit fields.
Reconciliation and multi-row budget balancing should commit atomically. Existing
Apps Script retry behavior is not sufficient for edits and reconciliation; implement
the stronger operation contract in section 4.2.

The Android client authenticates through server-mediated Pocket ID handoffs,
uses canonical API routes, and stores owner-scoped Room caches, drafts and
outbox operations. Stable UUIDv7 identifiers and revision guards preserve retry
safety. No permanent shared finance secret is embedded in the APK.

## 7. Cutover sequence

### Phase A — close design questions

- Use the test workbook to verify formula behavior, but use the owner-linked
  live workbook and the final frozen export for production row/status decisions.
  Repeat the redacted current-period formula/error scan at cutover.
- Record the historical and current FX selection rules and rounding behavior.
  Preserve the source workbook timezone as import provenance; use
  Europe/Madrid for the canonical business timezone and keep finance effective
  dates date-only.
- Determine whether full transaction history reconstructs every account
  balance. Define and verify any opening-state records required.
- Confirm the exact historical window required for budget and account reports.
- Import literal budget balances as frozen month snapshots, establish the
  opening balance before the first retained month, and verify the policy for
  historical edits across nonstandard formula months and reconciliation.
- Check unreconciled current-period transaction rows and the current-period
  budget pair for formula gaps/errors; preserve pre-boundary formula/error states
  as historical provenance.

### Phase B — schema and offline tools

- [x] Build the read-only Sheets export, SQLite dry-run/commit import, redacted
  verification, and SQLite export commands.
- [x] Add a provider-neutral SQLx schema scaffold using decimal text; it imposes
  no precision scale or rounding rule.
- [x] Generate checked-in synthetic formula/FX fixtures from the test workbook's
  formula templates. It contains synthetic finance values, generic helper
  labels, and source headers only.
- [x] Archive `Accounts!W:BE` and preserve dated historical account checkpoints
  with their source cell states.
- [x] Validate the migration and import/verify/export round trip against the
  sanitized fixture; the verifier reported source-row-exact parity.
- [x] Take a fresh live export after the source formula fix, import it to a
  disposable database, and verify source-row-exact transaction, account,
  checkpoint, budget, rate, valuation-provenance, opening-state, and quarantine
  coverage. All populated transaction rows are classified.
- [x] Owner-review `Transactions` row 8 as an opening-balance row. Preserve its
  original source account type and source cells; the fresh dry run now reports
  zero opening-balance warnings.

The test-workbook import dry run classified 2,822 dated transactions, 23
quarantined transaction rows, and 703 normalized source-oriented rate
observations. It reported 827 issue entries, so commit was refused. See
[`docs/canonical-db-phase-b-findings.md`](docs/canonical-db-phase-b-findings.md).

### Phase C — shadow parity

- [x] Import the sanitized formula fixture into a disposable SQLite database;
  the verifier reports exact source-row parity.
- [x] Add a DB read-model assembler for recent transactions, active real
  accounts, current budgets, and monthly budget history. It preserves imported
  source valuations and literal historical balances.
- [x] Add an offline, read-only calculation prototype for rational FX
  conversion, account/category deltas, reallocations, and open-month budget
  changes. Run eight synthetic scenarios without changing the database or a
  workbook.
- [x] Compare the assembled DB read model by value with the sanitized source
  snapshot for accounts, recent transactions and valuations, current budgets,
  and historical budget cells.
- [x] Compare the digest-bound 2026-09-30 owner-linked review snapshot by value
  for all active real accounts, current real budgets, all available historical
  budget cells, and the 200 most recent transactions with valuations. Source
  import verification also reports exact parity across every imported row.
- [x] Independently replay all archived `Transactions!J:Q` formulas from the
  archived transaction inputs, account currencies, and prior-rate tables.
  The replay matched every calculated output and reproduced every archived
  formula error; it made no workbook writes.
- [x] Compare post-transaction DB valuations with an independent sheet-formula
  calculation for eight sanitized scenarios (sixteen valuation sides), then
  compare the resulting account/category projections with fixed expected
  deltas and conservation/frozen-month invariants. No workbook writes occurred.

The separate test-workbook archive is a formula fixture, not the owner-linked
source of record. Its dry run has 827 unresolved validation entries across 713
rows, so importing and dispositioning that entire archive is not a migration
gate. Production-source parity is instead based on the digest-bound owner-linked
export, which passed exact source-row verification and read-model comparison.
Retain the sanitized fixture for repeatable calculation scenarios.

Repeat the archived formula replay with:

```sh
python3 scripts/sheets/verify-finance-formulas.py \
  --input .google/migration/finance-export-20260930T215347Z-db5a23ae
```

### Phase D — server API and both clients

- [x] Add GET `/api/db/finance` as a Pocket ID-authenticated, read-only DB
  snapshot endpoint. It returns canonical decimal text and remains disabled by
  default with `FINANCE_DB_READ_ENABLED=false`; `/api/finance` still reads from
  Sheets.
- [x] Implement `/api/budget/close` as an idempotent SQLite transaction that
  records a digest-bound snapshot of real category closing values and freezes
  the month atomically. It requires a month-end reconciliation covering every
  active real account and is disabled by default with
  `FINANCE_DB_CLOSE_ENABLED=false`.
- [x] Add `POST /api/db/category-correction` as a Pocket ID-authenticated,
  idempotent category-only correction for a transaction in a closed month. It
  applies signed expense deltas to the latest open budget snapshot, leaves all
  account balances and closed snapshots unchanged, flags the original in the DB
  read model, and is disabled by default with
  `FINANCE_DB_CATEGORY_CORRECTIONS_ENABLED=false`.
- [x] Add Pocket ID-authenticated transaction creation and guarded editing at
  `POST /api/db/transactions` and `/api/db/transactions/edit`, disabled by
  default with `FINANCE_DB_TRANSACTION_WRITES_ENABLED=false`. Creates and edits
  write operation and valuation revisions, update affected real-account native
  balances/current EUR display values, and apply expense, income, or
  reallocation changes to the latest open budget month. Edits require the
  transaction and account revisions, stay within that month, and currently
  accept only DB-created transactions. Imported rows remain protected pending
  review of the 15 source formula-reference exceptions. Closed-month
  corrections use the separate category-only operation. The native transaction
  editor calls these routes in canonical-database mode.
- [x] Add `POST /api/db/reconciliation` behind
  `FINANCE_DB_RECONCILIATION_WRITES_ENABLED=false`. It preserves the existing
  current-all-ledger reconciliation basis, checks expected account ledger
  revisions, replaces the one account/date adjustment while retaining each
  submission batch and transaction revision, and updates the latest actual
  account values atomically. The read model returns each account's current
  ledger revision for optimistic concurrency.
- [x] Make SQLite finance exports use a consistent read snapshot and add a
  digest-checked restore command for a fresh database target.
- [x] Exercise the local SQLite export and fresh-target restore against a
  disposable database; the drill verifies parity for all exported finance
  tables after restoring.
- [x] Integrate native transaction create/edit and browser DB reads/reconciliation.
  Pocket ID authorizes both clients through server-owned sessions.

- [x] Exercise transaction create/retry/conflict, edit/retry/stale-revision,
  reconciliation partial/full/retry/replacement/stale-revision, close
  prerequisite/retry/immutability, and closed-month write rejection against the
  sanitized fixture database. The test uses an inserted local Pocket session and
  a synthetic USD rate; it does not exercise a live Pocket ID login.
- [x] Add queue v2 backend-generation handling. Existing v1 operations are
  retained as Sheets work and are not replayed into the database.
- [x] Add a persistent owner-controlled Apps Script maintenance gate checked
  inside the same script lock as every finance mutation. It drains in-flight
  writes and blocks later calls through the updated deployment, including calls
  from older app builds.
- [x] Inventory the current Apps Script write targets. Source inspection found
  the archive write functions `submitTransaction`, `updateTransaction`, and
  `reconcileAccounts`, plus the direct Sheets CLI commands `sheets:append`,
  `sheets:patch`, and `sheets:migrate-live-schema`. A read-only Apps Script API
  listing found seven execution entries for the recorded live project:
  versions 1–5 and 7, plus one unversioned entry. The archive server config points to version 7.
  No live deployment was changed.
- [ ] During cutover, retire or fence superseded execution entries and review
  the unversioned entry, then restrict manual workbook edits. The script gate
  does not cover those paths. Keep direct Sheets CLI commits and manual
  workbook edits paused from the final export through cutover. The test
  transaction command targets only the separate test workbook.
- [x] Prepare and exercise local SQLite export and fresh-target restore tools.
- [x] Add a Pocket ID-authenticated, complete, consistent finance snapshot
  endpoint using the same 23-table interchange format as the admin export.
- [x] Store digest-verified encrypted generations on native mobile devices;
  retain the latest eight and show the last saved time and age.
- [x] Add a Pocket ID-authenticated restore path that accepts a verified
  snapshot only when all finance tables are empty and every finance write gate
  is disabled.
- [x] Exercise digest rejection, schema mismatch, non-empty target rejection,
  and exact fresh-target restore parity for all 23 finance tables against a
  sanitized disposable SQLite database.
- [x] Run a native encrypted-storage and fresh-database recovery drill on the
  available Android phone. Verify one surviving device copy before cutover;
  checking the second phone is a follow-up, not a cutover gate.
- [ ] As a lower-priority follow-up, build the best-effort DB-to-Sheets export.
  Surface export age and OAuth reauthorization needs; it does not gate entry or
  cutover.

### Phase E — production cutover

1. Stop new entry on the devices and browser.
2. Activate the Apps Script maintenance gate under its write lock, waiting for
   in-flight operations to finish. Confirm old clients cannot mutate Sheets and
   restrict manual edits. Keep SQLite finance writes disabled.
3. Take a consistent local SQLite backup and a final workbook export. Freeze FX
   inputs for extraction, wait for calculation completion, and verify source
   digests before/after extraction to reject a mixed-state export.
4. Import that complete export into empty production finance tables; rerun count,
   FX, account, and budget parity. Persist the manifest and initialize a
   verified full snapshot on the available phone before enabling writes. The
   other phone can receive and verify its snapshot on its next foreground sync;
   its availability is not a cutover gate. A Drive export is optional at
   cutover and does not gate entry.
5. Activate the DB backend generation for both clients. Keep the Apps Script
   gate closed until an explicit rollback; old clients must fail safely.
6. Monitor redacted errors, duplicate retries, DB outbox age, FX lookup misses,
   balance/report parity, each device's last verified snapshot generation as
   it becomes available, and last successful off-host backup age. Preserve the
   original source workbook as the cutover archive; update only the separate
   backup workbook through the optional export process.

## 8. Fallback: SQLite to Google Sheets

Rollback is a controlled source-of-truth switch, not ongoing dual writing.
Keep the frozen workbook as a rollback target until two month-end closes have
been reconciled in the new system, capped at 90 days after cutover. After that
window, retain it as an archive only.

### Preconditions

- Stop new client entry and resolve committed and pending DB operations using
  operation receipts.
- Then pause database write endpoints and background writers and wait for
  in-flight transactions. Keep the Apps Script maintenance gate closed.
- Back up SQLite and export a full, verified database snapshot with a cutover
  watermark.
- Confirm the frozen source workbook still matches the recorded sheet/schema
  fingerprints and has enough prepared formula rows.

### Restore behavior

Add a Rust command to export DB state to a private, versioned interchange file
and a Sheets restore command, for example `sheets:restore-finance`. The Sheets
command should default to dry run and verify every target before writing.

- For imported transactions, verify the original row fingerprint, then restore
  the latest DB input values and ID/metadata to that same row, including edits
  made after cutover. Do not append a duplicate or restore only metadata.
- For DB transactions created after cutover, locate by transaction ID if
  already present; otherwise use the existing serialized gateway or a
  lock-protected restore endpoint to write permitted input cells to a prepared
  row.
- Restore the literal `Currency` and `To Amount` only when the database
  records those as explicit overrides. Preserve formula cells and formula
  fingerprints for all other rows. If an edit removed an override, reinstate
  the validated row-relative formula from the prepared template in D or E.
- Restore actual account balances and reconciliation records in a
  lock-protected batch. Preserve signed/zero reconciliation adjustments.
- Restore any rate data or rate-version inputs needed to reproduce saved
  transaction valuations. If the frozen workbook cannot reproduce the
  database's rates, stop and restore to a prepared clone with the required
  rate data rather than claiming rollback succeeded.
- Persist restore-run IDs and per-row expected-before/expected-after fingerprints
  off-host. On resume, accept an exact already-applied result or apply an exact
  before-state; stop on any third state. Reconcile these checkpoints after a
  timeout rather than duplicating a write. Keep clients fenced during partial
  restores and verify all rows before marking the run complete.
- Verify recalculated account native balances, EUR views, corrected budget
  history, and conversion values before directing clients back to Sheets.
  Persist the restored backend generation, route compatible clients to it, and
  only then reopen the gateway. Never allow both backends to accept writes.

Only restore validated D/E formulas when removing explicit overrides; preserve
all other transaction formulas. Never generic-append beyond the prepared area. The existing sheet contract requires formula fingerprints and
preserved input/formula ownership ([workbook contract](docs/workbook-contract.md)).

## 9. Acceptance criteria

The migration is ready only when all of these hold:

- Both clients use one authenticated server API; neither writes finance data
  directly to Apps Script during normal operation.
- Retrying each operation with the same UUID produces its recorded result,
  including edits/reconciliation after a lost response and later ledger changes;
  changed payloads using the same UUID are rejected.
- Obsolete archive writers cannot write after the freeze, and obsolete backend
  generations are rejected.
- Imported transaction counts and source-row coverage match the workbook, and
  all old formulas/literal overrides are represented.
- Historical J:Q formula gaps and errors retain source-state provenance.
  Unreconciled current-period transactions and the current-period budget pair
  have no unresolved formula gaps or errors before they feed live totals.
- All historical transaction EUR/native values match the sheet's evaluated
  values within the agreed exact storage/rounding policy.
- Current account native balances, actual balances, and EUR display values
  match; historical budget snapshots match for every imported month/category.
- The app reproduces the allocated-budget-versus-actual-account-funds
  consistency check from canonical data; it does not depend on the workbook's
  Accounts R8 cell.
- Future budget results match for expenses, income, reallocation, overspending,
  budget balancing, and month boundaries.
- Test cases cover EUR/PHP/USD transactions; source-currency override;
  same- and cross-currency transfers in both currency directions; actual
  received amount differing from a spot-rate estimate; missing/weekend dates;
  reconciliation with positive, negative, and zero variance; stale edits; and
  duplicate batch retries; historical edits and backdated entries across closed-month snapshots and
  effective-base changes; mid-month cutover; concurrent/repeated reconciliation;
  and edits
  whose response is lost before cutover.
- SQLite backup and restore have been exercised against a disposable copy.
- Each mobile device stores complete, encrypted, digest-verified generations
  with schema version, table counts, and an operation watermark; a fresh SQLite
  finance database can be recovered from either device snapshot, including
  operation receipts, revisions, valuations, and import mappings. A physical
  drill on the available Android phone verified capture, survival across an app
  restart, and restore parity. The second phone's physical check is follow-up,
  not a cutover gate. A partial or invalid sync never replaces its last
  verified generation.
- The optional Drive exporter records its last successful generation and reports
  expired Testing-mode authorization clearly. It is best-effort and does not
  gate finance writes or cutover; no 24-hour off-host recovery target is claimed
  while its grant is expired.

## 10. Operational and privacy requirements

- Keep the finance database on the persistent production volume. Continue
  consistent local SQLite backups using the SQLite backup API, including before
  deployment and cutover; do not copy a live `.sqlite` file without its WAL state.
- **Mobile snapshots (first backup priority):** after each successful foreground
  sync, retain a complete and consistent finance database generation on that
  device. Include the schema version, table counts, content digest, and operation
  watermark. The Pocket ID-authenticated `/api/db/finance/backup` endpoint and
  native app capture/store path are implemented. The app encrypts snapshots at
  rest with a per-device, per-owner Android Keystore-protected key, keeps the latest eight
  verified generations, and promotes a generation only after digest and
  encrypt/decrypt verification. It shows the latest verified time and warns
  when it is more than 24 hours old. The mobile restore API requires the exact
  same schema version, an empty finance table set, and all finance write gates
  disabled; startup rejects restore mode if any database write gate is enabled.
  After a restore, disable restore mode before re-enabling finance writes. The
  server-side recovery drill passed against the sanitized disposable fixture.
  A physical drill on the available Android phone passed encrypted capture,
  local verification, persistence across force-stop/relaunch, and restore into
  a fresh disposable database with exact parity across 23 finance tables and
  345 rows. It used a synthetic test session, so it did not validate live
  Pocket ID login. The other phone's physical check is follow-up, not a cutover
  gate. Device copies are independent recovery points, but simultaneous loss or
  erasure of both phones and the server can lose finance data.
- **Optional off-host backup (lower priority):** make one-way exports to a
  separate, access-restricted Sheets workbook when the household owner's OAuth
  grant is valid. Keep the OAuth consent screen in Testing as requested; its
  refresh token expires after seven days, so surface failures and prompt owner
  reauthorization. Do not block Pocket ID entry on this grant. The exporter is
  best-effort, and the prior 24-hour off-host recovery target is not guaranteed.
- If implemented, authorize the optional Sheets exporter as the household owner
  and pin it to that account. Prefer a dedicated `drive.file` grant for the
  separate backup workbook, stored separately using the server's encryption at
  rest. Keep the consent screen in Testing; the owner accepts reauthorization as
  needed. The existing `spreadsheets` scope reaches every spreadsheet in the
  account, so prefer a per-file grant if that can be done without substantial
  additional setup.
- Export from a consistent DB read snapshot with a schema version, generation,
  counts, content digests, and watermark. Include every finance table required
  for recovery: ledger and revisions, literal/formula flags, exact valuations and
  rates, budget baselines/corrections, opening states, reconciliation history,
  operation receipts, reference data, and import/identity manifests. Store exact
  decimals, IDs, and serialized metadata as literal text in dedicated backup
  tabs; do not rely on live sheet formulas to preserve canonical values.
- Write a new staging generation, read it back and verify it, then mark it
  complete. Retain at least the previous seven verified daily generations and
  the cutover archive; prune only after a newer generation verifies. Preflight
  workbook capacity and partition exports if necessary. Partial syncs must leave
  the last complete generation recoverable. Clients never write to backup tabs.
- Prove recovery from this workbook into a fresh DB before cutover. The normal
  ledger/formula view may support rollback through section 8, while dedicated
  backup tabs preserve data the legacy workbook cannot represent. Disaster
  recovery fences surviving clients, restores operation receipts, and resolves
  outstanding queue operations before reopening writes.
- This is an off-host **finance** backup, not a complete server backup. Auth
  tokens/sessions are excluded from Sheets; host recovery recreates deployment
  configuration and requires sign-in again using credentials maintained outside
  the host. Document and rehearse that recovery path as part of the restore drill.
- **Longer term:** upload encrypted, consistent SQLite backup files and manifests
  to Google Drive, with retention, integrity checks, and a tested download/restore
  path. This is a follow-up improvement, not a prerequisite for this migration.
- Keep Google credentials, exported finance data, manifests with source IDs,
  and restore bundles outside Git. Extend `.gitignore` before creating them.
- Admin tools must redact personal descriptions, balances, rates, and user
  identity from logs and errors by default. Require an explicit private report
  path for sensitive diagnostics.
- Retain import and export manifests long enough to support rollback, then
  securely remove temporary files according to the household's retention
  preference.
- Keep SQLite migrations additive and deploy-safe. Do not drop the auth tables
  or change token/session storage as part of the finance migration.

## 11. Owner decisions and remaining implementation gates

The following owner decisions are settled:

1. **Money policy:** preserve imported amounts, rates, and historical valuations
   at source precision; use canonical decimal text and exact decimal arithmetic
   without intermediate rounding. Round new posted amounts at the currency's
   valid posting precision using midpoint rounding to even, and format displayed
   money to two decimals. The live source scan found variable input and
   formula-result scales, so imported values do not use a global fixed integer
   scale.
2. **FX source:** fetch new daily reference observations directly from the ECB;
   use the exact or most recent prior published rate for a transaction date.
   Allow entries with the latest stored rate and flag new valuations as stale if
   the feed is more than three TARGET operating days behind. Keep pre-cutover
   rates and valuations frozen. Automatically refresh valuations in open months when an
   applied ECB observation is corrected, preserving the previous valuation in
   history; never refresh a closed month's valuation. Actual transfer received
   amounts remain explicit inputs. Apply a 1.02 source-side factor to new
   non-EUR input amounts as a proxy for suboptimal exchange rates and possible
   foreign transaction fees. Preserve imported historical valuations as-is.
3. **Budget history:** April 2025 is the first budget snapshot; pre-April carry-in
   is not inferred. A month remains open to correction until the explicit
   `Close month` action after end-of-month reconciliation. Account
   reconciliations alone do not close it. After close, a backdated correction
   does not change that month's close or any later closed-month budget balance.
   A correction recorded in a closed month has no account-balance effect; its
   audited category adjustment applies in the current open month and flags the
   original transaction.
4. **Account opening state:** use the explicit May 1, 2025 opening-balance rows
   for real accounts, preserving signed values. Negative credit-card seeds are
   always less than or equal to zero. The owner marked SB Platinum invalid for
   opening-balance use; retain it inactive for history and quarantine its seed.
5. **Rollback window:** retain the frozen workbook as a rollback target until two
   month-end closes are reconciled in the new system, capped at 90 days after
   cutover; then it becomes an archive.
6. **Backups:** prioritize full, verified, encrypted recovery snapshots on each
   mobile device. Keep a separate DB-to-Sheets export as a lower-priority,
   best-effort off-host copy, authorized by the household owner's Google OAuth
   account in Testing mode; the owner accepts reauthorization when its token
   expires. Do not block Pocket ID finance entry on the Drive grant. Because
   OAuth expiry can leave Drive exports stale for more than 24 hours, no fixed
   off-host recovery-point target is promised. Encrypted SQLite uploads to Drive
   remain a later improvement.

The owner has settled the finance policies for money rounding, FX inputs,
historical budget closes, account opening balances, rollback, backups, and
Pocket ID authorization for the database. Phase C projection parity is
complete. Phase D write routes, Pocket ID sessions, native/browser integration,
and backend-aware queue handling are implemented and enabled in production. The
disposable-database write/close and local SQLite recovery drills pass. The
available Android phone's native snapshot/restore drill also passes. Production import parity, the deployment and Apps Script fencing controls, live
Pocket ID reads, and the available phone’s verified production snapshot have
passed. Canonical DB writes are enabled. The live real-entry propagation and refreshed phone snapshot checks passed;
second-phone and first live statement reconciliation follow-ups are tracked in
the cutover handoff. The separate Drive
export is not a cutover gate.

The 2026-09-30 live export is digest-checked and its corrected current-period
budget formula has a value. The shadow import classifies explicit real-account
opening rows separately from transactions and retains reviewed exclusions in
quarantine. The remaining negative opening-seed warning was owner-reviewed as
an opening-balance row without changing its source account type; the fresh dry
run reports zero validation issues or review warnings. Recovery into a fresh
database from the available phone's verified snapshot has passed; a second
phone check is not a cutover gate. After cutover, the app must use the database
as the only canonical writable store.

## 12. Implementation boundaries

Phase B adds read-only export tooling, an offline importer/verifier/exporter,
sanitized fixtures, and a provisional schema scaffold. Phase C adds an offline
projection model but does not move client reads or writes. The noisy separate
test-workbook archive is not a production import source or a required migration
gate. The live export confirms the repaired current-period budget formula, and
the source values support canonical decimal text with exact arithmetic. The
server has default-off finance read and month-close endpoints, plus private
SQLite export and fresh-target restore commands. Disposable close and restore
drills pass. Remaining calculation work includes ECB rate
revisions and automatic open-month valuation refreshes. Base production import
decisions on the frozen live workbook export. Production cutover was authorized and its import/parity and local/device recovery
gates passed on October 1, 2026; retain the frozen workbook and consistent
pre-import SQLite backup for rollback. See
[`docs/canonical-db-phase-c.md`](docs/canonical-db-phase-c.md) for Phase C
results and [`docs/canonical-db-phase-d.md`](docs/canonical-db-phase-d.md) for
Phase D disposable drill results and remaining gates.


## Post-cutover budget model — October 2, 2026

The owner selected Proposal 2: calculate open-month balances from verified
opening carryover, effective monthly allocations, ledger valuations, budget
reallocations, and category corrections. Calendar months appear automatically;
explicit closes remain immutable and occur in order after valid reconciliation.
See [the maintained model](docs/budget-model.md), which supersedes earlier
open-month mutable snapshot and three-day reallocation behavior in this plan.

Proposal 3, a separate budget ledger with explicit funding and potentially an
unallocated-funds category, is a future direction. It has not been implemented.
