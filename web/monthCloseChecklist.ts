import type { FinanceSnapshot } from './contracts';
export type CloseCheck = { id: string; label: string; detail: string; ready: boolean; required: boolean; page: 'budgets' | 'reconcile' | 'overview'; month?: string };
export function monthCloseChecklist(snapshot: FinanceSnapshot, currentMonth: string): CloseCheck[] {
  const month = snapshot.budgetMonth;
  const status = snapshot.budgetHistory?.find(item => item.month === month)?.status;
  const prior = snapshot.budgetHistory?.filter(item => item.month < month && item.status === 'open').sort((a, b) => a.month.localeCompare(b.month))[0];
  const batch = snapshot.reconciliationStatus.batch;
  const reconciled = !!batch?.isComplete && batch.isCurrent !== false && batch.effectiveDate.slice(0, 7) === month;
  const deficits = snapshot.budgets.filter(item => item.remainingEur !== null && Math.round(item.remainingEur * 100) < 0);
  const missingBalances = snapshot.budgets.some(item => item.remainingEur === null);
  const expectation = snapshot.budgetCoverage?.expectations.find(item => item.month === month);
  const outstanding = expectation?.outstandingEur ?? snapshot.incomeExpectation.outstandingEur;
  return [
    { id: 'period', label: 'Month has ended and is open', detail: status !== 'open' ? 'This month is already closed or imported history.' : month >= currentMonth ? 'Close this month after it ends.' : 'This month is available to close.', ready: status === 'open' && month < currentMonth, required: true, page: 'budgets' },
    { id: 'earlier', label: 'Earlier months are closed', detail: prior ? 'Close ' + prior.month + ' first; edits there can change carryover.' : 'No earlier open months.', ready: !prior, required: true, page: 'budgets', month: prior?.month },
    { id: 'reconciliation', label: 'All accounts have a fresh reconciliation', detail: batch?.isCurrent === false ? 'Account activity changed. Reconcile again.' : reconciled ? 'Every active account is included and the check is current.' : 'Reconcile every active account for this month.', ready: reconciled, required: true, page: 'reconcile', month },
    { id: 'deficits', label: 'Review category deficits', detail: missingBalances ? 'Some category balances are unavailable. Refresh before closing.' : deficits.length ? deficits.length + ' categories are below zero. Balance them or deliberately retain their deficits.' : 'No categories are below zero.', ready: !missingBalances && !deficits.length, required: false, page: 'budgets' },
    { id: 'income', label: 'Review outstanding expected income', detail: outstanding > 0.005 ? 'Some planned income is still outstanding. Record receipts or review whether to complete the expectation.' : 'No expected income remains outstanding for this month.', ready: outstanding <= 0.005, required: false, page: 'overview', month },
  ];
}
