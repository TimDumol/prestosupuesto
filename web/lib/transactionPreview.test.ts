import assert from 'node:assert/strict';
import test from 'node:test';
import { transactionPreview } from './transactionPreview.ts';

test('salary settles outstanding funding once and only excess credits Unallocated', () => {
  const result = transactionPreview({ kind: 'income', amount: 2200, currency: 'EUR',
    toAccount: { name: 'Checking', currency: 'EUR' }, incomeTreatment: 'salary', outstandingEur: 2000 });
  assert.ok(result.includes('Checking increases by €2,200.00.'));
  assert.ok(result.includes('Category allocations stay unchanged.'));
  assert.ok(result.includes('Expected salary decreases by €2,000.00; Unallocated increases by €200.00.'));
});

test('partial salary, extra income, and refunds have distinct budget effects', () => {
  assert.ok(transactionPreview({ kind: 'income', amount: 500, currency: 'EUR', incomeTreatment: 'salary', outstandingEur: 2000 })
    .includes('Expected salary decreases by €500.00; Unallocated increases by €0.00.'));
  assert.ok(transactionPreview({ kind: 'income', amount: 40, currency: 'EUR', incomeTreatment: 'unexpected' })
    .includes('Unallocated increases by €40.00; category allocations stay unchanged.'));
  assert.ok(transactionPreview({ kind: 'income', amount: 40, currency: 'EUR', incomeTreatment: 'refund', toCategory: 'Clothing' })
    .includes('Clothing remaining budget increases by €40.00.'));
});

test('foreign transfers use the entered received amount and never guess an exchange rate', () => {
  const result = transactionPreview({ kind: 'transfer', amount: 100, currency: 'EUR', toAmount: 105,
    fromAccount: { name: 'Checking', currency: 'EUR' }, toAccount: { name: 'Wallet', currency: 'USD' } });
  assert.ok(result.includes('Checking decreases by €100.00.'));
  assert.ok(result.some(line => line.startsWith('Wallet increases by ') && line.includes('105.00')));
  assert.ok(result.includes('Category budgets stay unchanged.'));
  const foreignExpense = transactionPreview({ kind: 'expense', amount: 100, currency: 'USD', fromCategory: 'Travel' });
  assert.ok(foreignExpense.some(line => line.includes('the EUR value of')));
});

test('budget moves leave accounts untouched and corrections/edits avoid misleading deltas', () => {
  assert.deepEqual(transactionPreview({ kind: 'reallocate', amount: 20, currency: 'EUR', fromCategory: 'Travel', toCategory: 'Food' }),
    ['Travel decreases by €20.00.', 'Food increases by €20.00.', 'Account balances stay unchanged.']);
  const correction = transactionPreview({ kind: 'expense', amount: 20, currency: 'EUR', correction: true, editing: true, fromCategory: 'Food' });
  assert.ok(correction.some(line => line.includes('replacement, not the difference')));
  assert.ok(correction.some(line => line.includes('correction rules')));
  assert.ok(!correction.some(line => line.includes('remaining budget decreases')));
  assert.deepEqual(transactionPreview({ kind: 'expense', amount: NaN, currency: 'EUR' }), []);
});
