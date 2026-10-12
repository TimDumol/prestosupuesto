import assert from 'node:assert/strict';
import test from 'node:test';
import { monthCloseChecklist } from './monthCloseChecklist.ts';
import type { FinanceSnapshot } from './contracts.ts';

const snapshot = {
  budgetMonth: '2026-09', budgetHistory: [{ month: '2026-09', status: 'open' }],
  reconciliationStatus: { batch: { isComplete: true, isCurrent: true, effectiveDate: '2026-09-30' } },
  budgets: [{ name: 'Food', remainingEur: 10 }], incomeExpectation: { outstandingEur: 0 },
} as FinanceSnapshot;
const requiredReady = (value: FinanceSnapshot, current = '2026-10') => monthCloseChecklist(value, current).every(item => !item.required || item.ready);

test('closing requires an ended open month and a matching complete/current reconciliation', () => {
  assert.equal(requiredReady(snapshot), true);
  assert.equal(requiredReady(snapshot, '2026-09'), false);
  for (const batch of [null, { isComplete: false, isCurrent: true, effectiveDate: '2026-09-30' }, { isComplete: true, isCurrent: false, effectiveDate: '2026-09-30' }, { isComplete: true, isCurrent: true, effectiveDate: '2026-08-31' }]) {
    assert.equal(requiredReady({ ...snapshot, reconciliationStatus: { ...snapshot.reconciliationStatus, batch } } as FinanceSnapshot), false);
  }
  assert.equal(requiredReady({ ...snapshot, budgetHistory: [{ ...snapshot.budgetHistory![0], status: 'closed' }] }), false);
});

test('earlier open months link to the oldest blocker', () => {
  const value = { ...snapshot, budgetHistory: [...snapshot.budgetHistory!, { month: '2026-08', status: 'open' as const, categories: [] }, { month: '2026-07', status: 'open' as const, categories: [] }] };
  assert.equal(requiredReady(value), false);
  assert.equal(monthCloseChecklist(value, '2026-10').find(item => item.id === 'earlier')?.month, '2026-07');
});

test('deficits and outstanding income are review warnings, not invented server requirements', () => {
  const value = { ...snapshot, budgets: [{ ...snapshot.budgets[0], remainingEur: -20 }], incomeExpectation: { ...snapshot.incomeExpectation, outstandingEur: 100 } };
  assert.equal(requiredReady(value), true);
  assert.deepEqual(monthCloseChecklist(value, '2026-10').filter(item => !item.ready).map(item => item.id), ['deficits', 'income']);
  const covered = { ...value, budgetCoverage: { expectations: [{ month: '2026-09', outstandingEur: 0 }] } } as FinanceSnapshot;
  assert.equal(monthCloseChecklist(covered, '2026-10').find(item => item.id === 'income')?.ready, true);
});
