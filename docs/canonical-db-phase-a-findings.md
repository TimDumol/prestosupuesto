# Canonical database plan: Phase A findings

**Inspected:** 2026-09-28

**Status:** The original test-workbook scan and a separate live current-month
scan are complete. No September 2026 transaction formula gaps or errors were
found in the live workbook. Remaining design work is report-window, precision,
and historical-edit parity.

The original formula inventory below records a read-only scan of the configured
**test workbook**. The separate live check below was made against the workbook
URL supplied by the owner. Both sections omit account names, descriptions,
amounts, balances, and rates. The live check read formula metadata and calculated
results only; it made no workbook writes and retained no exported finance data.

## Phase A outputs

| Design question | Finding | Status |
| --- | --- | --- |
| Transaction, FX, budget, and account formulas | Test formulas are recorded below; live September rows were checked separately. | Live current-period transaction and budget scans are clean |
| FX selection, rounding, and timezone | Test-workbook scan found exact-or-prior weekly-rate lookups and no explicit rounding calls; its timezone was Asia/Manila. Canonical household timezone is Europe/Madrid. | Test behavior recorded; effective finance dates are date-only; numeric parity still open |
| Test-workbook account reconstruction | All 34 real accounts with numeric balances match the filtered transaction-ledger calculation. | Test snapshot reconstructs current balances; history edge cases remain |
| Test-workbook budget and account report history | Test snapshot headers span 2025-04 through 2026-10; ordinary transaction dates span 2025-05 through 2026-08. | Required retained/reporting window not yet decided |
| Budget history and closing boundary | Owner confirms literal remaining cells freeze closed month balances when a category's monthly base changes. Eleven nonstandard formulas and the pre-history opening state remain to verify. | Literal-snapshot rule accepted; opening state and unusual formulas open |
| Reconciliation policy | Existing gateway uses the current all-ledger balance and a selected month-end adjustment date. | Existing behavior recorded; DB parity case open |
| Legacy phone queues | Owner considers the legacy phone queues drained for this migration; no device inventory or queue-drain gate is required. | Closed assumption |
| Accounts R8 cross-check | It compares total allocated budget with actual funds across accounts; owner says to perform this check in the app. | Sheet formula is not a migration blocker |

## Live workbook current-month verification

Read-only scan of the owner-linked live workbook on 2026-09-28, using
Europe/Madrid as the current month:

- The live Transactions tab has 122 dated September rows, spanning rows
  3108–3229 and dates 2026-09-01 through 2026-09-28.
- None of those 122 rows has a missing formula in J:Q or a calculated formula
  error anywhere in A:T.
- The September budget pair is Expenses v3 columns AL:AM; neither column has a
  calculated error.
- The live ledger does have older formula errors, all outside September:
  #N/A in M/N at rows 357, 441, 546, 634, 933, and 1156; #VALUE! in Q at
  row 1148; #REF! in Q at row 1506; and #VALUE! in D and J:N at row 1878.
  Per the owner's direction, these pre-September entries are reconciled history.
  Preserve their source error/formula states; they are not current-period
  cutover blockers.
- The live budget scan found #VALUE! in Expenses v3 U37, V24, and V36; these
  are outside the September AL:AM pair. The live Accounts scan found one #N/A
  at R8. The owner clarified that R8 checks total allocated budget against
  actual funds across accounts; perform that check in the app instead.

This is a live-workbook result and supersedes the test-workbook date range and
test formula-error counts for current-period decisions. It does not change the
test-only findings retained below for historical/import design.

## Test-workbook settings and transaction ledger

The test-workbook scan reported timezone `Asia/Manila`, locale
`en_US`, and recalculation `ON_CHANGE`, consistent with the existing
[workbook contract](workbook-contract.md#lookup-and-derived-sheets). The
canonical household timezone is `Europe/Madrid`. Finance effective dates are
calendar dates; import them without timezone conversion. Preserve the source
timezone only as provenance. Use Europe/Madrid for application month boundaries
and current-date decisions; store audit instants in UTC.

The transaction sheet has 3,024 rows with input data through row 3034. Of
these, 3,001 rows have dates in the ordinary reporting range, 22 populated rows
have no date, and one row has an out-of-range numeric date (row 2169). Blank
date rows are 3003–3011, 3013–3019, 3021–3025, and 3027. Do not invent
dates for these rows or let them define the reporting window. Preserve them in a
quarantined import-review set until their inputs and any effect on ledger totals
are understood. The row with the out-of-range numeric date also requires review;
do not silently move it into the ordinary history.

Transaction conversions use the transaction date and these lookup rules:

- `XLOOKUP` uses match mode `-1` (exact match or next smaller rate date) and
  search mode `2` (binary search over ascending dates). The rate date columns
  therefore need to remain sorted ascending. This is the live behavior; an
  earlier cached report incorrectly reflected linear search mode.
- Source-side PHP and USD conversions in `Transactions!K` and some
  `Transactions!M` branches multiply the selected rate by `1.02`. The direct
  EUR conversion in `Transactions!P` has no such multiplier. Account summary
  conversion uses `TODAY()` and does not apply that multiplier.
- `Transactions!L`, `N`, and `Q` multiply their source amount/conversion
  fields. The target must preserve the exact branch and direction of each
  conversion, including the source-side adjustment, until an intentional
  product-rule change is approved.

The `Exchange Rates` sheet requests weekly Google Finance series for PHP/EUR
and USD/EUR from 2025-04-20, and PHP/USD from 2025-03-01. The observed returned
series were respectively 75 rows (2025-04-25 through 2026-09-26), 75 rows
(2025-04-26 through 2026-09-26), and 82 rows (2025-03-07 through 2026-09-25).
The formula interval `7` means weekly, per [Google's GOOGLEFINANCE reference](https://support.google.com/docs/answer/3093281).
The workbook specifies the series start dates, but the first returned weekly
row can be later. The inspection does not establish a replacement rate source
or updater.

There are transaction output formula-pattern holes in populated rows. The
locations below identify J:Q cells without formulas. Their raw contents are not
included; a missing formula is not automatically a defect or a sanctioned
manual override. D/E have the documented input-override cases.

| Output column | Populated rows without a formula |
| --- | --- |
| J | 635, 1230, 1553, 2397, 2412, 2770, 3011, 3023, 3024 |
| K | 635, 1230, 1553, 3011, 3023, 3024 |
| L | 635, 750, 751, 1230, 1553, 3011, 3023, 3024 |
| M | 635, 1230, 1553, 3011, 3023, 3024 |
| N | 347, 635, 1230, 1553, 3011, 3023, 3024 |
| O and P | 635, 1230, 1553, 3011, 3023, 3024 |
| Q | 635, 1230, 1553, 3011, 3023, 3024 |

Six rows (635, 1230, 1553, 3011, 3023, and 3024) have gaps across most or all
of J:Q, which points to a row-level formula-template gap worth checking as a
group. The remaining gaps are isolated: N347, L750:L751, and J2397/J2412/J2770.
This is a structural observation only; it does not establish whether any source
cell is intentionally literal or missing a required calculation.

The formula error scan found `#N/A` in six `Transactions!M` cells, six
`Transactions!N` cells, and one `Accounts!Q` cell; `#VALUE!` in one transaction
row across `D`, `J`, `K`, `L`, `M`, and `N`, one `Transactions!Q` cell, and
`Expenses v3!U` plus two `Expenses v3!V` cells; and `#REF!` in one
`Transactions!Q` cell. These counts describe calculated formula errors, not
the private inputs that caused them. The six M errors and six N errors may be
on overlapping rows; the scan does not identify them as 12 independent
transactions. The single row with errors across D and J:N is a distinct input
or calculation anomaly. The Q errors include both a value error and a broken
reference. Do not infer a cause from the error token alone.

### Test-workbook transaction formula-hole and error disposition

The missing-formula inventory is evidence about workbook structure, not proof
that the corresponding transaction is wrong. D and E have documented literal
override cases, but no such general override rule is documented for J:Q. Keep
the semantic D/E override flags separate from the per-cell J:Q source state.

For every populated row with a J:Q formula hole or formula error, the final
frozen export should retain the source input, formula text (if present), raw
evaluated value/type, and row fingerprint. Build a private exception register
keyed by row and column; record whether each cell is a formula, literal, blank,
or error, whether the row contributes to a real account/category, and the
reviewed disposition. Do not print private inputs or amounts in the report.

Recommended disposition:

1. Compare each hole with the row's operation shape, currencies, adjacent
   prepared-row formula pattern, and the formula's referenced rate/date. A
   blank or literal output is not automatically an error and must not be
   silently filled with a new API valuation.
2. Preserve numeric pre-cutover evaluated values and frozen historical rates
   exactly as imported. If a required historical valuation is genuinely absent
   or erroneous, repair it only from a reviewed source/evidence and record a
   migration correction with provenance. Do not rewrite the frozen workbook.
3. Do not activate a transaction with an unresolved valuation error if it
   affects an account balance, budget expense, or EUR report. Resolve it from
   source evidence or keep it quarantined with an explicit owner-approved
   exclusion and verified aggregate treatment. A non-impacting error may remain
   in provenance if that non-impact is demonstrated.
4. Treat the Accounts Q error as an account-summary conversion issue: native
   balance reconstruction can still be checked separately, but EUR summary
   parity needs an explicit current-rate result before cutover.

The six M/N #N/A cells, the D/J:N #VALUE! row, the Q #VALUE! and #REF!
cells, and the Accounts Q #N/A cell are test-workbook review items only. The
scan alone cannot establish their causes. The live September scan found no
transaction formula errors or J:Q gaps. Under the owner's reconciliation rule,
pre-September cases are preserved with their source states and do not block
cutover; any newly active transaction or current-period budget error must still
be resolved before it is used in live totals.

The test-workbook Expenses v3 U/V errors are separate from the live check.
The live errors and September pair are recorded in the section above; preserve
closed historical error states rather than silently converting them to zero.

## Test-workbook account reconstruction and history

The account balance formula applies the workbook's own transaction date
criteria (`Date > Accounts!T1` and `Date <= Accounts!T2`). That includes 3,001
ordinary dated transactions, excludes the one out-of-window date, and leaves
the 22 blank-date rows outside the calculation. Among 40 numeric account
summary rows, 34 are real accounts and all 34 match the transaction-derived
balance. Two helper-account rows differ; the existing workbook notes identify
missing transfer-side amounts in helper rows as a known limitation. No opening
seed is required to reconstruct the current balances of the 34 real accounts
under this calculation.

This result establishes current balance reconstruction, not every historical
account report. The exact historical account-report window and treatment of
any legacy opening-state rows still need to be specified. The transaction
ledger's ordinary date range is 2025-05 through 2026-08; that observed range is
not, by itself, a product decision about how much history to retain.

## Test-workbook rounding and numeric precision

The test-workbook scan covered 38,228 formulas across `Transactions`,
`Exchange Rates`, `Accounts`, and `Expenses v3`. It found no calls to
`ROUND`, `ROUNDUP`,
`ROUNDDOWN`, `MROUND`, or `TRUNC`. The formulas therefore contain no explicit
rounding at those points. This does not settle displayed precision, binary
floating-point behavior, or the database decimal scale. Choose those only
after comparing unformatted workbook results with the proposed database rules
for representative expenses, transfers, and FX conversions.

## Test-workbook budget history and exceptions

The test workbook's budget sheet has 33 category rows and 19 month headers
from 2025-04
through 2026-10. The 2026-10 column is blank. Across its 627 remaining-balance
cells, 103 are formulas (92 match the ordinary rollover pattern, 11 use other
formulas), 460 are literal entries, and 64 are blank. These are counts only;
no category or monetary values are included here.

For 437 literal cells after the initial month where prior remaining and
current expense inputs were available, 215 equal the ordinary calculation
`current base + prior remaining - current net expense`, and 222 differ. The
owner confirms that literal-ifying these cells is the workbook's mechanism for
freezing a month's remaining balance when the category's monthly base changes;
otherwise the changed base would alter prior balances. The differences should
therefore be imported as intentional closed-month snapshots, not classified as
resets or additive adjustments. Unless another source provides the former
monthly base amounts, treat the pre-cutover historical bases as unknown; record
the cutover-time base with its effective month instead of copying it backward.
In the first month, 20 numeric literal cells
were comparable to `base - expense`: 5 match and 15 do not. This is consistent
with an opening carry-in balance, but the exact pre-2025-04 opening state still
needs to be established.

The documented model has 27 real categories and six helper categories. Its
ordinary remaining formula is:

```text
current base + prior remaining - current net expense
```

Net expense includes the transaction category flows described in the
[budget notes](sheet-understanding-review.md#4-categories-and-accumulating-budgets).
The observed month headers show workbook coverage, not the exact history the
application must report. Establish the pre-2025-04 opening snapshot. Treat the
literal balances as frozen closes; do not classify them as resets or additive
adjustments. Do not project the current budget base backward unless former base
amounts are independently available. For historical transaction edits, verify
that correction deltas flow through later closes while budget-base changes
remain effective only from their selected month.

## Reconciliation policy

The existing gateway reads the current calculated native balance, writes the
entered actual balance, and creates or replaces a reconciliation adjustment
for the selected month-end. When replacing the same account/date adjustment,
it adds that old adjustment back before calculating the replacement. The
adjustment is the resulting balance minus the entered actual balance. This is
the current all-ledger balance basis, not a historical as-of-month-end
statement calculation. The canonical database plan adopts this behavior;
verify it with a database parity case before cutover.

Evidence: [gateway reconciliation behavior](workbook-contract.md#month-end-reconciliation)
and `reconcileAccounts` in `apps-script/Code.js`.

## Phone queue assumption

The owner considers the legacy phone queues drained for this migration. No
phone inventory, queue payload inspection, or queue-drain maintenance window is
a Phase A or cutover prerequisite. The mobile outbox remains part of the
post-cutover offline design.

## Remaining Phase A decisions

- Confirm the retained/reporting history and opening state before the first
  imported month.
- Select numeric storage and display rules, then verify them against
  unformatted values for representative expenses, transfers, and FX conversions.
- Select and document a public daily FX provider. Keep pre-cutover rates and
  reconciled transaction valuations frozen as imported values.
- Verify the adopted reconciliation behavior in a DB parity case, including
  replacing an existing same-date adjustment.
- Preserve historical formula gaps/errors as provenance. Recheck only
  unreconciled current-period rows and the current-period budget pair for
  cutover blockers; the live September scan currently passes those checks.

The test-workbook scan and live current-month check are complete. Both were
read-only. This note records formula rules, row/cell locators, structural
counts, aggregate parity, and status decisions without financial values.
