# Website UX backlog

Review date: 2026-10-02. Scope: the browser analysis workspace in `web/` and its canonical finance API. Findings came from reviewing the implementation, not an authenticated visual audit. Mobile changes are outside this work.

## High priority — implemented

- [x] **Find older transactions.** Replace the ledger's globally limited recent-row filtering with an authenticated, month-filtered, paginated query. Provide search across all history, exact category/account filters, matching counts, retry, and distinct loading/error/empty states.
- [x] **Explain category and account balances.** Link categories and all active accounts to their filtered ledger. Show category carryover + allocation − net spending + adjustments = remaining where those components are available. For frozen history, explicitly explain when a component breakdown is unavailable. Account details identify current book balances separately from the reporting period's activity.
- [x] **Guide month closing.** Show a checklist for an ended open month, earlier open months, complete/current reconciliation, category deficits, and outstanding expected income. Link to the unfinished steps. Required checks gate closing; financial review warnings require acknowledgment without changing the server's existing closing rules.
- [x] **Preserve transaction drafts.** Store new and edited drafts by household user, reporting month, and entry. Reopening restores incomplete input. Closing a form retains its draft; explicit discard and successful save remove it. Persist the create ID to make retries safe across reloads. Detect revision mismatch for restored edits. Report storage failure and warn before leaving if work cannot be saved.
- [x] **Support pasted local amounts.** Accept decimal commas, decimal points, valid European/US grouping, grouped spaces, and currency labels. Show the interpreted value and validation beside the field. Preserve arithmetic formulas and reject amounts with more than two decimal places instead of silently rounding. A single comma or point is treated as decimal (so `1.234` is not interpreted as 1234); use `1.234,00` or `1,234.00` for grouped amounts.

## Medium priority

- [x] **Clear save and freshness feedback.** Confirm budget moves with their resulting balances. Show the last refresh time, stale state, and distinguish a failed write from a successful write followed by a failed refresh.
- [x] **Investigate reconciliation discrepancies.** Link account discrepancies to transaction history, allow notes, and help inspect missing expenses, duplicates, transfers, and fees before applying an adjustment.
- [x] **Preview budget moves.** Show source/destination balances before and after. Warn when a move causes another deficit and offer a reviewable multi-donor balancing proposal.
- [x] **Manage scheduled allocations.** Edit/cancel scheduled changes and show which plan applies in each month.

## Additional use cases to explore

- [ ] Split a single purchase among several budget categories.
- [ ] Recurring bills and transaction templates; expected payments not yet recorded.
- [ ] Link full/partial refunds and reimbursements to original expenses, including expenses in closed months.
- [ ] Record cross-currency transfers and their separately categorized fees together.
- [ ] Multiple household income streams, irregular income, and installment receipts.
- [ ] **Irregular expenses and savings targets.** Set a target amount and date for annual insurance, holidays, repairs, or other infrequent spending. Show progress from accumulated category balances and suggest the monthly contribution needed to reach the target.
- [x] **Accidental duplicate entry.** Warn when separately created transactions share an account, amount, date, and similar description, including when both household members record the same purchase. Keep the warning optional so legitimate repeated purchases remain possible. This is separate from retry deduplication.
- [x] Household attribution, understandable edit history, and friendly conflict recovery.
- [ ] Bank CSV import previews with duplicate detection, and filtered transaction export.

## Smaller UX improvements

- [x] Put reporting month and ledger filters in the URL for shareable views and reload restoration.
- [x] Explain all disabled Save buttons with field-specific guidance.
- [x] Replace technical product copy such as “canonical database” and “accounting flow” with user-facing terms.
- [x] Review narrow-screen font sizes, navigation labels, touch targets, and table/chart presentation in a browser.

## Validation and boundaries

The high-priority work includes tests for localized monetary parsing, draft isolation/restoration/storage failure, closing checklist readiness, and SQL pagination/search/filtering beyond 200 transactions. Build and type checking cover the web integration; the Rust suite covers existing server behavior. An authenticated visual/browser audit remains a separate follow-up. No production deployment or financial writes are part of this implementation.


Verification commands:

- `npm --prefix web test`: 16 tests passed.
- `npm run web:typecheck`: passed.
- `npm run web:build`: passed (existing bundle-size advisory).
- `cargo test --manifest-path server/Cargo.toml --bin presto-finance-server`: 13 passed; two existing fixture-dependent drills ignored.
- `git diff --check`: passed.

During verification, the existing income-settings test needed a same-origin header to reach the authentication check. It then exposed a connection-pool deadlock: exchange rates were read through the pool while a write transaction held its sole connection. Rate reads now use that transaction, and the existing persistence test passes.

## App UX improvements — 2026-10-02 follow-up

Scope includes mobile and web. Reconciliation discrepancy actions, explanations of unavailable actions, and failed-sync recovery are excluded from this follow-up at the owner's request.

- [x] Share localized amount/formula parsing across clients and preview interpreted mobile amounts.
- [x] Persist unfinished mobile entries per user and backend; resume after navigation/restart and explicitly discard drafts.
- [x] Compact funding summaries with expandable calculations; put web categories needing attention before detailed funding and charts.
- [x] Preview account/category effects before saving, including income treatment and currency-transfer amounts.
- [x] Put optional correction fields under More details.
- [x] Add Save and add another for entering multiple receipts.
- [x] Use visible phone navigation labels and clearer household-facing copy.
- [x] Show transaction month/date explicitly and keep category Remaining beside its name.

The additional use cases remain proposals except accidental duplicate detection and household attribution/edit history, implemented in the website follow-up below. Irregular expenses/savings targets remain unimplemented.

Follow-up validation: mobile/web TypeScript checks, the entry unit suites, and all 16 web unit tests pass. The desktop/phone Playwright suite includes 30 visual and interaction checks, including a browser-backed harness for the mobile draft hook. Updated screenshots were visually reviewed. No native device run or production deployment was performed.

## Website medium-priority follow-up — 2026-10-02

Implemented all medium-priority and smaller website items, plus accidental duplicates and household attribution/conflict recovery. Mobile is outside this follow-up.

- Budget moves preview both balances, warn about a resulting deficit, and confirm the calculated balances after saving. Multi-donor proposals are reviewed and recorded one move at a time, updating after each save; insufficient funds are explained.
- The header shows refresh time and flags data older than five minutes or a failed refresh. Saved writes retain success feedback when their subsequent refresh fails.
- Reconciliation discrepancies link to the affected account's ledger in the closing month, with guidance for missing expenses, duplicate purchases, transfers and fees. Investigation notes persist per household user/account/closing date on this device; they are not shared or part of a posted adjustment.
- Scheduled category allocations can be edited in their effective month or cancelled before taking effect. Cancellation preserves other changes and resumes the previous plan. A collapsible list shows the plan applying in the selected month. Changes already effective are replaced through the current-month editor.
- Duplicate preflight checks page through the selected month's account history, compare date/account/currency/amount and normalized descriptions, and show an optional confirmation identifying existing purchases and their household author. Retry IDs are excluded. Simultaneous saves before either entry is visible can still evade this advisory check.
- Ledger entries show their household author and lazily loaded, authenticated version history with time, amount, description, date, and account/category names. New versions retain display names; older versions use the available session name or recorded identity. Conflicts preserve drafts, retrieve the latest entry, and offer explicit review choices before saving again.
- Reporting month and ledger search/type/account/category/history scope persist in the URL. Save forms show missing-field guidance. Household-facing copy and phone input sizes, touch targets and table presentation were reviewed using authenticated synthetic browser fixtures.

Validation: 18 web unit tests, TypeScript checking, production build, the Rust suite (13 passed; two existing fixture-dependent drills ignored), and all 46 desktop/phone Playwright checks passed. Updated desktop/phone screenshots were visually reviewed. No production deployment or real household financial writes. Existing bundle-size and Rust deprecation advisories remain.
