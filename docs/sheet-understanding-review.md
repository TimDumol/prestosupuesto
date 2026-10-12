# Spreadsheet understanding — review and confirmation

Status: **owner-confirmed; test implementation in progress**  
Based on: read-only inspection of the production workbook and structural plus
controlled-write testing in its test copy  
Last reviewed by the tooling: 2026-08-23

This is my current understanding of how the finance workbook behaves. It is
written as a functional description rather than a formula inventory. Please
mark incorrect assumptions, add exceptions, or answer the questions at the end
before the Android app is allowed to write production data.

No spreadsheet IDs, transaction descriptions, account balances, or budget
amounts are included in this document.

## 1. Overall model

`Transactions` is the authoritative event ledger. The account, budget, and
reporting sheets are derived views over that ledger. The app should therefore
write a transaction and then read the recalculated results; it should not write
calculated balances directly.

Each transaction describes two simultaneous flows:

- a **category flow** from `From Category` to `To Category`; and
- an **account flow** from `From` to `To`.

This supports ordinary spending and income as well as transfers between
accounts and reallocations between budgets.

**Please confirm:**

- [Y] `Transactions` should remain the primary and authoritative ledger.
- [Y] The app must never write calculated budget or account balances.
- [Y] New app transactions should use the existing `Transactions` tab, not
      `EU Transactions` or another historical/import tab.

## 2. Transaction columns

The current `Transactions` schema is:

| Column | Meaning | Writer |
| --- | --- | --- |
| A `Date` | Effective transaction date | App/user |
| B `Description` | Free-text description | App/user |
| C `Amount` | Amount leaving the source side | App/user |
| D `Currency` | Normally derived from the source account; literal transaction currency in the rare override case | Formula or app override |
| E `To Amount` | Amount arriving at the destination | Formula, except cross-currency transfers |
| F `From Category` | Source category | App/user |
| G `To Category` | Destination category | App/user |
| H `From` | Source account | App/user |
| I `To` | Destination account | App/user |
| J-Q | Source/destination currencies, dated rates, native amounts, and EUR amounts | Formulas |

For ordinary transactions, E contains `=C[row]`. For a cross-currency account
transfer, the app supplies both amounts: C is the amount removed from the source
account and E is the amount credited to the destination account.

The ledger has formulas and validations prepared through row 3800. A writer
must fill the first unused prepared row and preserve its formulas. It must not
perform a generic append after row 3800.

**Please confirm:**

- [N] C is always expressed in the source account's currency. -- In rare cases, C can be a different currency, in which case the approximate equivalent in source account's currency is used in calculations. This is usually a temporary situation (when the actual deduction isn't known yet)
- [Y] E is always expressed in the destination account's currency.
- [Y] The user should enter E only when the source and destination currencies
      differ.
- [Y] The transaction date, rather than entry time, determines historical
      exchange-rate conversion.

## 3. Supported operations

| App operation | From category | To category | From account | To account |
| --- | --- | --- | --- | --- |
| Expense | Selected budget | `Expense` | Selected real account | `Expense` |
| Income | `Income` | Selected budget | `Income` | Selected real account |
| Account transfer | `Balance Transfer` | `Balance Transfer` | Selected real account | Selected real account |
| Budget reallocation | Selected budget | Selected budget | `Reallocation` | `Reallocation` |

The test copy successfully recalculated all four operations, including both
same-currency and cross-currency account transfers.

My interpretation of an income row is that it both credits an account and adds
money to a chosen budget. If income can instead be entered without immediately
assigning it to a budget, that needs another mapping.

**Please confirm:**

- [Y] Every expense is assigned to exactly one budget and one real account.
- [Y] Every income entry is immediately assigned to exactly one budget and one
      real account.
- [Y] Account transfers must not change any real budget.
- [Y] Budget reallocations must not change any real account.
- [Y] Negative amounts are not a normal input mechanism; refunds/corrections
      need an explicit UX rule.

## 4. Categories and accumulating budgets

The category source is `Expenses v3!A3:A35`, also exposed through the named
range `Categories`. There are currently 33 rows: 27 marked `IsReal = 1` and six
system/helper categories marked `IsReal = 0`.

My proposed UI rule is:

- show only `IsReal = 1` categories in ordinary user pickers;
- use system categories such as `Expense`, `Income`, and `Balance Transfer`
  internally according to the operation mapping; and
- do not let the user rename or create categories in the first app version.

`Expenses v3` stores a base monthly budget in EUR and a PHP reference amount,
followed by an `Expense` / `Remaining` pair for each month. For ordinary budget
rows, the rollover formula is conceptually:

```text
remaining this month
  = base monthly budget
  + remaining from the previous month
  - net expense this month
```

Net monthly expense is calculated from the category side of `Transactions` in
EUR. Spending from a category increases its expense; reallocation into a
category reduces its net expense; income assigned to a category also increases
the amount remaining.

Some historical cells contain manual starting values or exceptional formulas.
The app should consume the sheet's calculated current result rather than try to
reimplement historical rollover logic.

**Please confirm:**

- [Y] `IsReal = 1` is the correct test for a category users may select.
- [Y] The EUR budget in column B is the recurring amount added every month.
- [Y] Unspent remaining budget rolls forward indefinitely, including negative
      remaining amounts.
- [Y] Income assigned to a category should increase that category's remaining
      budget.
- [Y] Reallocation may move more than the source category currently has
      remaining; the app does not prevent this.

## 5. Accounts and reconciliation

`Accounts!A:D` defines account name, type, `IsReal`, and native currency.
System/helper accounts coexist with real bank, cash, and card accounts.

For each account, the sheet derives:

- credits from transaction destination amounts;
- debits from source amounts converted to the source account's native currency;
- calculated native balance as credits plus debits;
- a manually maintained actual balance;
- reconciliation difference between calculated and actual balance; and
- calculated and actual balances converted to EUR using a current rate.

My proposed UI rule is to show only `IsReal = 1` accounts in user pickers and to
reserve `Expense`, `Income`, and `Reallocation` for internal mappings.

Owner correction: real accounts can become unavailable for new transactions.
The approved implementation adds `IsActive` in column R, the first fully free
column before the historical-balance section. Column P was rejected because it
intersects existing merged warning cells. User pickers require both
`IsReal = 1` and `IsActive = TRUE`; inactive accounts remain valid on historical
transactions and in reports.

**Please confirm:**

- [N] `IsReal = 1` is the correct test for an account users may select. -- There are some accounts no longer in use, we probably can add another column for that IsActive
- [Y] The app may display reconciliation differences but should not change the
      manually maintained actual balance yet.
- [Y] Credit-card balances and signs should be displayed exactly as calculated
      by the sheet, without app-side sign reversal.
- [Y] The first release does not need an account-reconciliation workflow.

## 6. Currency conversion

`Exchange Rates` supplies historical PHP/EUR, USD/EUR, and PHP/USD rates. The
transaction formulas use the row's effective date and source/destination
currencies to populate D and J:Q. Account summaries separately use a current
rate for their present EUR view.

The app should display the sheet's converted results and should not duplicate
the exchange-rate formulas. A cross-currency transfer requires the actually
received destination amount because that value may include bank spreads or
fees and need not equal a simple market-rate conversion.

**Please confirm:**

- [Y] EUR is the common reporting/budget currency.
- [Y] PHP, USD, and EUR are the only currencies needed in the first release.
- [Y] The existing historical-rate formulas, including their current
      adjustments, should be treated as authoritative.
- [Y] For cross-currency transfers, the user knows and enters the actual
      destination amount.

## 7. Proposed hidden transaction metadata

A full read of `Transactions!R:T` in the test copy returned no values through
row 3800. I propose using and hiding these columns:

| Column | Header | Purpose |
| --- | --- | --- |
| R | `Transaction ID` | App-generated UUID; stable identity and retry deduplication |
| S | `Created At` | UTC timestamp recording when the app submitted the row |
| T | `Created By` | Stable app user/device label for audit and troubleshooting |

These columns are now installed and hidden in the test copy. They are metadata
only and do not participate in the existing calculations. Existing rows remain
blank. Every new app row gets a UUIDv7,
allowing a timed-out request to be retried without creating a duplicate.

**Please confirm:**

- [Y] R:T may be added to the test copy and hidden.
- [Y] The same columns may later be added to production after test approval.
- [Y] `Created By` should contain: td_prestosupuesto
      (for example a first name, Google account identifier, or anonymous device
      label).
- [Y] Existing historical rows do not need UUIDs immediately.

## 8. Concurrent writes and the gateway

Choosing the next prepared row and writing it are separate operations. If two
phones submit together without coordination, both can choose the same row.

The proposed Apps Script gateway will:

1. authenticate an allowed app user;
2. acquire one script-wide lock;
3. reject a duplicate `Transaction ID` or return its existing result;
4. verify the headers and prepared formula template;
5. choose the next prepared row;
6. write only A:C, F:I, R:T, plus E for cross-currency transfers;
7. flush/recalculate and return the resulting transaction;
8. release the lock even if an error occurs.

The test implementation uses Google sign-in plus an Apps Script API executable
restricted to signed-in users, with a second allowlist check inside the gateway.
No permanent shared secret is embedded in the APK.

**Please confirm:**

- [Y] Only the two household users should be able to submit transactions.
- [Y] Both users have Google accounts that can be used for sign-in.
- [Y] Both users already have, or may be granted, access to the workbook.

## 9. Cost and capacity expectation

As of 2026-08-23, `LockService` has no separately metered price. Apps Script is
governed by execution quotas, and Google says standard Sheets API usage is
available at no additional cost. The standard Sheets API quota is currently 60
read requests and 60 write requests per minute per user per project, far beyond
the expected usage of this two-person app.

Google's current documentation says usage beyond the standard API quotas is
planned to become billable later in 2026. We will remain inside the standard
quota and will not request a paid quota increase or enable a separately billed
hosting service without explicit approval.

Current official references:

- [Apps Script quotas](https://developers.google.com/apps-script/guides/services/quotas)
- [Google Sheets API quotas and pricing](https://developers.google.com/workspace/sheets/api/limits)
- [Apps Script LockService](https://developers.google.com/apps-script/reference/lock/lock-service)

## 10. Known exclusions for the first integration

Unless you say otherwise, I will treat these as out of scope for the first
working version:

- deleting existing transactions (editing is now supported by the guarded gateway);
- reconciliation and changing actual account balances;
- category/account administration;
- modifying budget formulas or historical exchange rates;
- importing transactions in bulk;
- attachment/receipt storage; and
- replacing Google Sheets with another database.

## Owner corrections

Please add any corrections or exceptions here, or send them in chat:

1. Use UUIDv7
2. _________________________________________________________________________
3. _________________________________________________________________________
