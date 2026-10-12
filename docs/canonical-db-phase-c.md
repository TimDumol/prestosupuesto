# Canonical database: Phase C shadow calculations

**Status:** Phase C shadow parity is complete for the digest-bound owner-linked
review archive, the checked-in sanitized fixture, and the representative
formula-reference projection scenarios below. The app still uses Sheets, and
the default-off Phase D DB read endpoint is not routed to clients. No
production workbook or finance database was written during these comparisons.

## What is implemented

- `server/src/finance_model.rs` performs FX conversion with exact rational
  arithmetic through the final currency posting precision. It selects the
  exact or latest prior observation, records the applied rate dates and exact
  multiplier fractions in its result, flags observations more than three
  TARGET operating days old, and projects account, category, reallocation, and open-month
  budget deltas.
- `server/src/finance_read_model.rs` assembles recent transactions, active real
  accounts, the current budget, and historical budget months from SQLite. It
  reads imported valuations and historical remaining balances as stored.
- `presto-admin finance shadow-simulate` runs against an imported sanitized
  fixture only. It is read-only and refuses an ordinary finance import. The
  simulation covers EUR expenses and income, a cross-currency transfer with a
  received amount that differs from its spot estimate, a reallocation, PHP and
  USD source-currency overrides, and negative and zero reconciliations. For
  each of eight scenarios, it also recalculates both valuation sides with a
  separate formula-reference implementation and compares the post-rounding
  amounts with the database model.
- `scripts/sheets/verify-finance-formulas.py` independently recalculates the
  archived `Transactions!J:Q` formula cells from transaction inputs, account
  currencies, and prior-rate tables. It honors each stored formula's actual
  cell references and Google Sheets' case-insensitive text matching. It prints
  aggregate counts and does not write to the workbook.
- The model applies the owner-approved 1.02 source-side FX adjustment to new
  non-EUR input amounts as a proxy for suboptimal rates and possible foreign
  transaction fees. Imported historical valuations continue to use their saved
  workbook results.

## Shadow run

The checked-in sanitized fixture was committed to a disposable SQLite database
under `/tmp`. The source verifier reported exact parity for seven transaction
rows, fourteen side valuations, 56 J:Q source cells, fourteen budget snapshots,
six normalized rate observations, and all 210 account-checkpoint cells.

Recreate and run the same local shadow sequence with:

```sh
cargo run --manifest-path server/Cargo.toml --bin presto-admin -- finance import \
  --input fixtures/canonical-db/finance-export \
  --database /tmp/presto-phase-c-finance.sqlite --commit --allow-test-workbook
cargo run --manifest-path server/Cargo.toml --bin presto-admin -- finance verify \
  --input fixtures/canonical-db/finance-export \
  --database /tmp/presto-phase-c-finance.sqlite
cargo run --manifest-path server/Cargo.toml --bin presto-admin -- finance shadow-simulate \
  --input fixtures/canonical-db/finance-export \
  --database /tmp/presto-phase-c-finance.sqlite --as-of 2026-09-08
```

The import refuses to merge into an already-populated finance schema. Use a
fresh disposable path for a repeat run. The simulation opens that database
read-only and requires the sanitized fixture marker in its import manifest.

For an owner-linked review archive that has a clean import plan, compare the
database-backed read model to the source archive by value with:

```sh
cargo run --manifest-path server/Cargo.toml --bin presto-admin -- finance compare-read-model \
  --input .google/migration/finance-export-20260930T215347Z-db5a23ae \
  --database /tmp/presto-phase-c-live.sqlite --as-of 2026-09-30
```

This command requires an already-imported database containing exactly one
matching archive. It opens SQLite read-only, requires the saved source identity
map, compares the current real-account and budget read models, all available
historical budget category cells, and the 200 most recent transactions with
their stored valuations. It reports aggregate counts only.

The offline projection then completed eight representative scenarios (sixteen
valuation sides), projected six ledger accounts and two real budget categories,
and confirmed that all sixteen sides match the independent sheet-formula
calculation. It also
confirmed that closed-month projections retain their imported values. Its
read model loaded seven transactions, two active real accounts, two current real
budget categories, and two available budget months.
Transfer categories remain helper categories, reallocation conserves the total
real-category budget, prior-rate selection works, stale observations are
flagged, and negative/zero reconciliation inputs are accepted. The command
reported that it changed zero database rows and made zero workbook writes.

Before projection, the DB read model compared by value with the sanitized
archive: two active real account rows, seven recent transaction rows including
their source and destination valuations, two current budget categories, and
four historical category-month snapshots matched. The eight scenario results
also matched explicit expected account and category deltas in the offline
projection.

The sanitized fixture's source verifier and read-model comparison use its
saved synthetic workbook snapshot. Separately, the digest-bound 2026-09-30
owner-linked review archive was imported into a disposable database, verified
with exact source-row parity, and compared to the DB read model. The comparison
matched all 14 active real account rows, 27 current real budget
categories, 486 historical category-month cells, and 200 recent transactions
including valuations. The import had zero validation issues and one opening
balance type warning at first; row 8 was later owner-reviewed as an opening
balance, and a fresh dry run reports zero validation issues and zero review
warnings. The reviewed archive is a review snapshot, not a frozen cutover
export.

The archived formula replay completed with formula-result parity within the
numeric comparison tolerance:
30,326 calculated values matched, 19 formula-error cells were independently
reproduced as errors, and there were no mismatches. It identified 15 formula
cells with off-row or broken references across 9 rows, including one `#REF`.
Those formulas and their source values remain preserved in the imported
baseline. This replay checks the archived formula outputs; it does not prove
that new database projections match independently calculated workbook results;
the eight-scenario formula-reference comparison below supplies that separate
check.

The shadow simulator's independent formula-reference calculation derives its
FX amounts directly from synthetic EUR-based quotes, then compares both sides
with the DB model before checking the projected account/category deltas. It
does not open or write a Google workbook.
The separate test-workbook archive remains unimportable because its dry run has
827 unresolved validation entries across 713 transaction rows. It is a formula
fixture, not owner data or a required migration gate. Production-source parity
comes from the digest-bound owner-linked export, which passed exact source-row
verification and matched the DB read model by value. The sanitized fixture
remains the repeatable calculation scenario source.

Recreate the owner-linked formula replay with:

```sh
python3 scripts/sheets/verify-finance-formulas.py \
  --input .google/migration/finance-export-20260930T215347Z-db5a23ae
```

## Decisions and implementation gates

The owner selected **half to even** for midpoint rounding on new posted
amounts. The simulation uses that as its posting mode and still exercises
half-away-from-zero and toward-zero to preserve the comparison.

The owner also confirmed the source-side **1.02 FX uplift** for new non-EUR
input amounts, reflecting the assumption that foreign transactions may receive
suboptimal exchange rates and incur foreign transaction fees. Historical
imported valuations remain frozen.

Phase C's projection comparison is complete: the eight synthetic scenarios
matched across all sixteen valuation sides, then their account/category deltas
matched fixed expectations while preserving closed-month snapshots. No
Google workbook writes occurred.

Stale-rate age now counts TARGET operating days, excluding weekends and TARGET
closing days. The ECB says reference rates are usually updated each working day
except TARGET closing days, and in principle on all TARGET operating days
([ECB publication page](https://www.ecb.europa.eu/stats/policy_and_exchange_rates/euro_reference_exchange_rates/html/index.eu.html),
[ECB rate framework](https://www.ecb.europa.eu/stats/pdf/exchange/Frameworkfortheeuroforeignexchangereferencerates.et.pdf),
[TARGET calendar](https://www.ecb.europa.eu/paym/pdf/target/current/targetoverview.pdf)).
The sanitized shadow simulation checks Good Friday and Easter Monday are excluded.

The server now exposes `/api/budget/close` behind the default-off
`FINANCE_DB_CLOSE_ENABLED` flag. It requires an end-of-month reconciliation
batch covering every active real account, then writes the operation record and
digest-bound snapshot atomically. Identical retries return the saved response;
the closure and budget snapshot triggers prevent reopening or changing the
closed snapshot. The route remains disabled until the DB reconciliation write
path is available and its behavior is exercised against a disposable database.

Phase C is complete. Keep the shipped clients on Sheets until the Phase D
client build, disposable-database operation and recovery drills, and cutover
gates are ready. Review the 15 source formula-reference exceptions before
enabling edits to those imported transactions; their current formula results
remain the imported historical baseline.

Phase D now includes `GET /api/db/finance`, a Pocket ID-authenticated,
read-only snapshot endpoint that returns canonical decimal text. It is gated
by `FINANCE_DB_READ_ENABLED=false` by default; the existing `/api/finance` route
continues using Sheets for the existing client path until cutover.

Phase D also includes `POST /api/db/category-correction`, gated by
`FINANCE_DB_CATEGORY_CORRECTIONS_ENABLED=false`. It records a closed-month
correction against the original transaction, applies signed category expense
deltas to the latest open budget month, leaves account balances and closed
snapshots unchanged, and exposes `hasCategoryCorrection` on that transaction in
the DB read model. A positive `expenseDeltaEur` increases the category expense
and reduces its remaining amount; a negative value reverses that effect. The
operation is idempotent by UUIDv7 `operationId`; all delta amounts are rounded
to EUR cents using the owner-selected half-to-even rule. The route is
Pocket ID-authenticated and no client currently calls it.

Its request shape is:

```json
{
  "operationId": "01995d7a-0000-7000-8000-000000000001",
  "originalTransactionId": "imported-transaction-id",
  "reason": "Correct closed-month category allocation",
  "categoryDeltas": [
    { "categoryId": "category-a-id", "expenseDeltaEur": "-12.37" },
    { "categoryId": "category-b-id", "expenseDeltaEur": "12.37" }
  ]
}
```

Phase D now has `POST /api/db/transactions` and a guarded
`POST /api/db/transactions/edit` route. Both accept UUIDv7 operation and
transaction IDs, canonical account/category IDs, and decimal-text amounts.
Create stores the ledger row, initial transaction revision, and both valuation
sides atomically. It uses the stored prior ECB
rates, the approved 1.02 non-EUR source adjustment, and half-to-even cent
rounding. It updates real account native balances and recomputes their current
EUR display values using the latest stored prior rates; it updates expense,
income, or reallocation values in the latest open budget snapshot. Ordinary
entries are limited to that month. Edits require the expected transaction
revision and current ledger revisions for every real account involved. They
currently accept only DB-created transactions in the latest open month;
imported transactions remain protected until the 15 source formula-reference
exceptions have been reviewed. A description or category-only edit preserves
the saved valuations. Financial edits create a new valuation revision. Both
routes are gated by `FINANCE_DB_TRANSACTION_WRITES_ENABLED=false`. The native
client calls these canonical routes;
the app example keeps `sheets` selected by default. The browser client uses the
server's Pocket ID cookie for DB reads and reconciliation.

Example expense request:

```json
{
  "operationId": "01995d7a-0000-7000-8000-000000000011",
  "transactionId": "01995d7a-0000-7000-8000-000000000012",
  "kind": "expense",
  "date": "2026-09-15",
  "description": "Groceries",
  "amount": "12.37",
  "fromAccountId": "account-id",
  "fromCategoryId": "category-id"
}
```

The transaction create/edit and reconciliation routes have only been
compile-checked; they have not been enabled or exercised against a database.

The DB account read model now includes `ledgerRevision`. Migration 0009 adds
the counter to each account, and each DB transaction write increments the
revision for every real account it changes. A separate
`POST /api/db/reconciliation` route accepts an `expectedRevision` for each
account. It rejects stale submissions, applies the existing current-all-ledger
reconciliation rule, replaces the single account/date adjustment, appends a
transaction revision and immutable reconciliation batch, and updates the
account's latest actual/current balance in one SQLite transaction. Later
reconciliation submissions retain the earlier batch and revision history.
The route is gated by
`FINANCE_DB_RECONCILIATION_WRITES_ENABLED=false`; clients call it only in
canonical-database mode.

Example reconciliation request:

```json
{
  "operationId": "01995d7a-0000-7000-8000-000000000021",
  "date": "2026-09-30",
  "accounts": [
    {
      "accountId": "account-id",
      "actualBalance": "123.45",
      "transactionId": "01995d7a-0000-7000-8000-000000000022",
      "expectedRevision": 4
    }
  ]
}
```

All DB write routes and month close remain disabled by default. Their
disposable transaction, reconciliation, close, and local SQLite recovery
drills now pass; see [`docs/canonical-db-phase-d.md`](canonical-db-phase-d.md).
Those drills do not exercise the category-correction route, live OIDC login,
or the scheduled off-host Sheets backup.
