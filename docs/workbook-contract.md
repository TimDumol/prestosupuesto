# Existing workbook contract

This document records the schema discovered from the test copy of the existing
formula-rich workbook. It is intentionally free of spreadsheet IDs, transaction
descriptions, balances, and other personal values.

The production workbook and test copy had the same tab IDs, headers, named
ranges, formula count, validation count, locale, timezone, and recalculation
settings when inspected on 2026-08-23. All exploratory writes were made only to
the test copy.

## Transaction ledger

The canonical input ledger is `Transactions`. Its first row is a strict schema
fingerprint:

| Column | Header | Ownership |
| --- | --- | --- |
| A | Date | App/user input |
| B | Description | App/user input |
| C | Amount | App/user input; source amount |
| D | Currency | Sheet formula by default; app literal for the rare transaction-currency override |
| E | To Amount | Sheet formula, except a cross-currency transfer supplies the destination amount |
| F | From Category | App/user input |
| G | To Category | App/user input |
| H | From | App/user input; source account |
| I | To | App/user input; destination account |
| J-Q | Currency and converted amounts | Sheet formulas |

The existing formula template is already copied through row 3800. A safe writer
must use the first prepared row after the last populated input, leave every
formula cell alone, and stop if the formula or validation fingerprint differs.
It must not use a generic append operation that creates a row after the prepared
formula region.

Dates are written as Google Sheets numeric serial values using `RAW` input. This
avoids date parsing changing with workbook locale or the machine running the
script.

## Transaction mappings

| App operation | From category | To category | From account | To account | Amounts |
| --- | --- | --- | --- | --- | --- |
| Expense | Selected budget | `Expense` | Selected account | `Expense` | C = expense amount |
| Income | `Income` | Selected budget | `Income` | Selected account | C = income amount |
| Account transfer | `Balance Transfer` | `Balance Transfer` | Source account | Destination account | C = source amount |
| Budget reallocation | Source budget | Destination budget | `Reallocation` | `Reallocation` | C = reallocated amount |
| Cross-currency transfer | `Balance Transfer` | `Balance Transfer` | Source account | Destination account | C = source amount; E = destination amount |

Same-currency transfers leave E as its `=C[row]` formula. Cross-currency
transfers are the only operation currently allowed to replace E with a literal
value.

## Lookup and derived sheets

- The named range `Categories` points to `Expenses v3!A3:A35` and is the fixed
  category list used by transaction validation.
- The named range `AccountNames` points to `Accounts!A2:A1017`.
- `Accounts!A:D` contains account metadata. The calculated balance fields are in
  `G:M` and are read, never written, by the adapter.
- `Expenses v3` contains category rows and paired `Expense` / `Remaining`
  columns for each month. The adapter selects the pair whose date header matches
  the requested month; it does not hard-code a column letter.
- `Exchange Rates` contains historical rate formulas. Transaction formulas use
  the transaction date and account currencies to calculate the EUR and native
  amounts in J:Q.
- Workbook recalculation is `ON_CHANGE`, locale is `en_US`, and timezone is
  `Asia/Manila`.

## Verified formula behavior

Five tiny, labelled transactions were committed to the test copy: one expense,
one income, one same-currency account transfer, one budget reallocation, and one
cross-currency account transfer. For each case the tooling verified that:

- all formula cells not intentionally replaced remained byte-for-byte unchanged;
- category expenses and remaining balances moved in the expected direction;
- source and destination account balances moved in the expected direction; and
- dated exchange-rate formulas produced the expected native and EUR effects.

The rows remain in the test copy as an audit fixture. Their descriptions begin
with `[API TEST ...]`, and the private local audit is under
`.google/test-writes.jsonl`.

## App integration boundary

`scripts/sheets/finance-schema.mjs` is the pure mapping layer for the four app
operations. `scripts/sheets/test-transaction.mjs` is a deliberately test-only
writer, and `scripts/sheets/finance-snapshot.mjs` turns formula results into the
read model the app needs: recent transactions, active-month budgets, and account
balances.

The test copy now has hidden metadata columns `R:T` (`Transaction ID`,
`Created At`, `Created By`) and `Accounts!R` (`IsActive`). Existing transaction
rows remain blank in the metadata columns. New app IDs are UUIDv7. Account
pickers require both `IsReal` and `IsActive`; inactive accounts remain intact in
history and reports.

Concurrent writes are serialized by a bound Google Apps Script API executable:

1. acquire a script lock;
2. validate a request UUID and the schema fingerprint;
3. find the next prepared row;
4. write only the permitted input cells;
5. record the UUID for idempotency;
6. release the lock and return the calculated row.

The gateway authenticates the signed-in Google account, applies a two-person
allowlist, and is the only app component that chooses and fills a prepared row.
This keeps Google Sheets as the backing store and its existing formulas as the
calculation engine while keeping OAuth refresh tokens and service-account keys
out of the APK.
