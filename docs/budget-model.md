# Budget plan ledger, cash coverage, and immutable closes

Implemented October 2, 2026. This supersedes the mutable open-month snapshot
projection introduced during cutover. The household calendar is Europe/Madrid.

## Stored facts

- `budget_opening_state`: verified category carryover immediately before the
  first canonical month. Startup seeds this once from the immediately preceding
  imported historical snapshot, retaining source provenance. Missing openings
  produce an error rather than a guessed zero. Read-only reporting can use that
  same preceding snapshot if a seed has not yet been persisted.
- `category_budget_periods`: recurring EUR allocation settings with effective
  start months. A setting applies until the next setting for that category.
  Editing October does not delete a scheduled November setting. Settings changes
  have audit receipts in `finance_operations`.
- `finance_transactions` and latest `transaction_valuations`: ledger facts and
  their posted EUR values. Budget reads do not recalculate exchange rates.
- `budget_ledger_state`: activation month and explicit Unallocated opening,
  with the original category results and observed cash retained as provenance.
- `budget_ledger_movements`: immutable EUR movements for recurring funding,
  allocations, spending, reallocations, corrections, and income differences.
  Editing or deleting a source appends reversing or adjusting movements.
- `budget_plan_income`: monthly instances of recurring expected income, with
  a fixed EUR plan valuation and explicit completion receipts.
- `budget_income_receipts`: salary matching, unexpected income, refunds, and
  preserved pre-activation receipts. Native-currency salary amounts settle the
  expectation; differences from its EUR plan valuation adjust Unallocated.
- `budget_coverage_closures`: immutable cash, plan, income, and coverage figures
  captured at close, with a separate digest.
- Reallocation transactions: signed category funding effects, counted separately
  from spending and exactly once.
- `finance_category_corrections` and its items: auditable category adjustments in
  an open month; original transactions and frozen history remain unchanged.
- `budget_month_closures`: explicit immutable close records, including the full
  category calculation and its digest.
- `budget_month_snapshots`: imported provenance and historical results, plus
  compatibility copies of closing results. An imported snapshot for an open
  canonical month is not an authoritative balance and is never maintained by
  ordinary transactions.

The import manifest identifies the first canonical month. Earlier imported
history remains fixed, even where historical allocations are unknown.

## One calculation for open months

`server/src/budget.rs` calculates all open months in chronological order:

```
carryover = preceding month's remaining balance
adjustments = incoming reallocations - outgoing reallocations - correction expense deltas
available = carryover + applicable monthly allocation + adjustments
remaining = available - net transaction spending
```

Net spending uses outgoing category EUR amounts minus incoming category EUR
amounts, including refunds. After ledger activation, salary funds the existing
plan and unexpected income goes to Unallocated. Pre-activation income retains
its original category effect to preserve migrated balances. Reallocations are excluded from
spending. Account transfers have no category effect. Reconciliation transactions
use the system Reconciliation category and do not silently redistribute spending
budgets. Positive and negative category balances both carry forward.

The API exposes `carryoverEur`, `adjustmentEur`, `availableEur`,
`netSpendingEur`, `correctionEur`, and `remainingEur`. Existing clients retain
`expenseEur` (net spending including category corrections) and
`reallocationImpactEur` (positive for donor, negative for recipient).

Closed category results are read from the immutable close JSON, including the
monthly allocation and component breakdown captured at closing. Historical
imports can lack a component breakdown; it is reported as unknown.

## Calendar and lifecycle

Open months are calendar views, not rows that must be created by a scheduled job.
A request for November automatically evaluates November from October's result.
If several months were skipped, each allocation is applied once in its own month.
Planning a future allocation does not make that month available for posting.

- `historical`: imported month before canonical cutover; fixed.
- `open`: canonical month through the current Madrid month without a close.
- `closed`: explicitly frozen canonical month.

More than one month may be open. Canonical transactions may be entered, edited,
or deleted in any open month; an edit remains in its original month. Imported
source entries retain their existing edit restrictions. Reallocations have no
three-day deadline. Changes to an earlier open month flow through every later
open month's carryover. The API marks later balances with
`carryoverProvisional` while an earlier completed month awaits close.

The web UI shows status, provisional carryover, component values, scheduled
allocations, and a review step before closing. It can reconcile an older completed
month using today's observed balances and the recorded activity since that close.

## Reconciliation and closing

Reconciliation records account observations and adjustment transactions; it
neither starts nor closes a budget month. Closed periods reject new reconciliations.

Closing requires a completed month, chronological closing order, and a complete
reconciliation that is still valid for activity on or before that month's end.
Freshness uses finance-operation receipt order, including edits and deletions,
rather than timestamp ordering. Later-month activity does not invalidate a prior
month's reconciliation; backdated account changes do. Budget reallocations and
category-only adjustments do not invalidate account reconciliation.

Finance mutations obtain SQLite's writer lock before reading their dependencies.
Closing calculates and persists results and the immutable close atomically.
Operation receipts preserve retry behavior. Financial arithmetic remains decimal.

## Recovery and inspection

Schema version 14 expands the finance table set while retaining interchange
format version 1. Server exports, admin exports/restores, and encrypted phone
copies include the ledger tables. Earlier phone copies remain inspectable;
restoring to a live server requires its matching schema version.

Open results are reconstructable from explicit openings and ledger movements,
with source transactions, valuations, effective settings, and operation receipts
retained for inspection. Read-only reports do not materialize calendar months.

Read-only inspection:

```
presto-admin finance budget-report --database FILE --month 2026-10
```

A forecast can simulate November without changing the database or server clock:

```
presto-admin finance budget-report --database FILE --month 2026-11 --calendar-month 2026-11
```

Use `--category "Discretionary"` to limit report output. The forecast uses the
currently recorded facts and recurring plans, including expected funding; it
does not post future cash receipts or predict future spending.

## Budget ledger activation and migration

The server activates the ledger atomically in the current Madrid month after
applying schema migration 14. Existing category balances and historical closes
are preserved. Existing receipts retain their prior category effect; only
current-month receipts contribute to the initial current-month expectation.
Earlier historical expected income is not inferred.

Initial Unallocated is actual household cash minus existing assigned category
balances plus outstanding expected funding. Its explicit opening offsets the
current month's recurring funding and allocations so they are not counted twice.
Reconciliation changes cash coverage without rewriting this opening or budgets.

An explicit maintenance command uses the same activation path:

```
presto-admin finance budget-ledger-activate --database FILE --commit
```

Recurring funding and allocation movements are materialized under SQLite's
writer lock at startup, finance mutations, and finance requests. Repeating this
work appends only differences, so each month's funding and allocation is counted
once. A future read-only forecast evaluates recurring plans without posting them.

### Design discussion: anticipated income and cash coverage

The preferred direction is a separate budget plan ledger with actual cash
coverage. Anticipated income is an important part of the household's budgeting
workflow, so planned category allocations must be allowed to take effect before
the corresponding income arrives. A planned allocation is not an account
balance: expected income remains a forecast and must never increase actual cash.
Because planned assignments can exceed current cash, the plan ledger cannot
claim every assignment is backed by actual funds. Keep planned balances distinct
from actual cash, or represent forecast funding explicitly; an unallocated-funds
balance must not make forecast income look like cash.

The coverage view should distinguish actual account cash, remaining category
budgets, and projected coverage after outstanding expected income. The monthly
plan may exceed expected income; show the gap rather than rejecting the plan.
Late or short income changes coverage; completing a short income expectation
also reduces Unallocated without changing assigned category budgets.
Reconciliation differences should change actual cash and its coverage,
not category budgets. Initially, coverage should be shown for the household as
a whole; category-level funded balances would require an explicit rule for
assigning actual cash among categories.

### Agreed direction from the design discussion

- Recurring category allocations continue automatically, independently of when
  income arrives. Posted income increases actual account cash and can settle an
  expected-income item; receipt of expected income does not credit category
  budgets again. Refunds restore the original spending category's budget.
- Expected income can be linked to receipts. A partial or short receipt can leave
  the remainder outstanding or explicitly mark the expectation complete. The
  current household workflow normally has one income receipt per month.
- All currently modeled accounts count toward household cash coverage. Savings
  are modeled as budget categories, so savings account cash remains part of the
  pool backing those categories. Coverage must account for signed balances and
  avoid counting transfers between included accounts as new money.
- Funding gaps are informational. Existing account transaction validation remains
  in force; insufficient coverage does not add a new spending restriction.
- Reconciliation differences change actual cash and coverage. Any resulting
  category-budget adjustment must be explicit.
- Coverage starts at household level. An income shortfall does not automatically
  reduce or reprioritize category budgets; the user explicitly reallocates or
  adjusts the plan when desired.
- The ledger should retain as much useful history as possible: opening balances,
  recurring allocations, explicit adjustments and reallocations, spending,
  expected income, receipt matching and settlement decisions, reconciliation,
  and immutable close results. These facts are retained in the ledger,
  its source tables, operation receipts, and immutable closes.

### Unallocated and income settlement

Leftover planned funds belong to an Unallocated budget bucket. Expected income
adds planned funding, and category allocations move that funding from
Unallocated into the assigned categories. Unallocated may be negative when the
plan exceeds its funding. Its balance is distinct from actual cash coverage;
anticipated funding does not represent cash already received.

For example, expecting EUR 3,000 and allocating EUR 2,700 leaves EUR 300 in
Unallocated even before payday. Receiving the expected EUR 3,000 increases
actual cash and settles the expectation without another budget credit.

- Income above the expected amount increases Unallocated by the excess.
- Unexpected income increases Unallocated by the received amount.
- Marking a short income expectation complete reduces Unallocated by the
  unreceived remainder, allowing a negative balance. Assigned category budgets
  remain unchanged until the user explicitly adjusts them.
- A partial receipt with the remainder still outstanding retains that expected
  funding; coverage shows the current cash gap and outstanding expected income.
- Refunds increase actual cash and restore the original spending category's
  budget rather than increasing Unallocated.

### Expected income across immutable closes

Outstanding expected income carries forward when its expected month closes.
The close captures its outstanding amount as it stood at closing. A later
receipt is recorded in its actual receipt month and settles the original
expectation through an auditable link. It increases actual cash without creating
another planned funding credit or changing the earlier close.

Reports distinguish the frozen status at close from the expectation's current
settlement status. For example, October can retain EUR 200 outstanding at close
while the expectation now shows that EUR 200 was received in November.

If an outstanding remainder will never arrive, explicitly completing the
expectation records a reduction to Unallocated in the current open month,
linked to the original expectation. The earlier close remains unchanged.

The initial workflow should focus on one fixed salary receipt per month; partial
and late receipt handling follows these rules without requiring extra steps
for an ordinary fully received salary.

### Coverage and corrections after closing

Coverage includes all active real accounts with signed native balances converted
at the applicable account valuation date. Savings remain assigned categories.
Non-cash investments would require a separate coverage policy if introduced.

```
current coverage difference = actual cash - remaining assigned category budgets
projected difference = current coverage difference + outstanding expected income
```

Unallocated is shown separately, including any negative balance; it cannot hide
an assignment shortfall. Closed coverage freezes these components and the
expected-income states at closing. Current views show later settlement status.

Account activity discovered after its original month has closed is recorded as
an explicit expense or income correction in the current open month, linked with
`correctsTransactionId` to the original closed or historical transaction. Account
and category effects follow the selected expense or income treatment. Category
reclassifications retain the existing category-only correction operation. Neither
operation changes the original transaction or its closed month.
