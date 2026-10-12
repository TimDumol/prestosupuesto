import assert from 'node:assert/strict';
import test from 'node:test';
import { incomeComparison } from './incomeComparison.ts';

const base = { balance: 3000, budget: 5000, expected: 2000, recorded: 0,
  month: '2026-10', arrivalDay: 7, today: '2026-10-01' };

test('unreceived income explains the gap without changing actual cash', () => {
  assert.deepEqual(incomeComparison(base), {
    outstanding: 2000, dueDate: '2026-10-07', overdue: false,
    difference: -2000, adjustedDifference: 0,
  });
});
test('partial and full receipts replace expected income without double counting', () => {
  const partial = incomeComparison({ ...base, balance: 3800, recorded: 800 });
  assert.equal(partial.outstanding, 1200);
  assert.equal(partial.adjustedDifference, 0);
  const full = incomeComparison({ ...base, balance: 5000, recorded: 2000 });
  assert.equal(full.outstanding, 0);
  assert.equal(full.adjustedDifference, 0);
  const extra = incomeComparison({ ...base, balance: 5500, recorded: 2500 });
  assert.equal(extra.outstanding, 0);
  assert.equal(extra.adjustedDifference, 500);
});
test('income becomes overdue after the arrival date, including historical months', () => {
  assert.equal(incomeComparison({ ...base, today: '2026-10-07' }).overdue, false);
  assert.equal(incomeComparison({ ...base, today: '2026-10-08' }).overdue, true);
  assert.equal(incomeComparison({ ...base, today: '2026-11-01' }).overdue, true);
  assert.equal(incomeComparison({ ...base, today: '2026-10-08', recorded: 2000 }).overdue, false);
});
test('arrival dates are clamped for short months and leap years', () => {
  assert.equal(incomeComparison({ ...base, month: '2026-02', arrivalDay: 31 }).dueDate, '2026-02-28');
  assert.equal(incomeComparison({ ...base, month: '2028-02', arrivalDay: 31 }).dueDate, '2028-02-29');
});
test('cent rounding avoids residual gaps and handles zero expected income', () => {
  const result = incomeComparison({ ...base, balance: 0.1, budget: 0.3, expected: 0.2 });
  assert.equal(result.adjustedDifference, 0);
  assert.equal(incomeComparison({ ...base, expected: 0 }).outstanding, 0);
});
test('uses currency-aware outstanding income from the ledger even when EUR posting rates differ', () => {
  const result = incomeComparison({ ...base, expected: 6666.67, recorded: 8436.36,
    outstandingEur: 0 });
  assert.equal(result.outstanding, 0);
  assert.equal(result.adjustedDifference, -2000);
});
