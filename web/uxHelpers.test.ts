import { test } from 'node:test';
import assert from 'node:assert/strict';
import { balancingProposal, possibleDuplicates } from './uxHelpers.ts';
import type { GatewayTransaction, SubmitTransaction, GatewayBudget } from './contracts.ts';
test('duplicate warning matches household purchases while excluding retry identities and distinct payments', () => {
  const payload = { transactionId: 'new', kind: 'expense', date: '2026-10-01', description: 'Café supermarket', amount: 12.3, currency: 'EUR', fromAccount: 'Joint' } as SubmitTransaction;
  const row = { transactionId: 'other', kind: 'expense', date: payload.date, description: 'Cafe supermarket groceries', amount: 12.3, currency: 'EUR', fromAccount: 'Joint', toAccount: 'Expense', createdBy: 'Partner' } as GatewayTransaction;
  assert.equal(possibleDuplicates(payload, [row]).length, 1);
  for (const variant of [{ transactionId: 'new' }, { fromAccount: 'Personal' }, { amount: 12.31 }, { currency: 'USD' }, { date: '2026-10-02' }, { description: 'Train ticket' }]) assert.equal(possibleDuplicates(payload, [{ ...row, ...variant } as GatewayTransaction]).length, 0);
});
test('multi donor proposal uses cents, never overdraws donors and reports uncovered deficits', () => {
  const budget = (name: string, amount: number | null) => ({ name, remainingEur: amount } as GatewayBudget);
  const proposal = balancingProposal([budget('A', 10.1), budget('B', 2.2), budget('C', -11), budget('D', -3), budget('Unknown', null)]);
  assert.deepEqual(proposal, { moves: [{ from: 'A', to: 'C', amount: 10.1 }, { from: 'B', to: 'C', amount: 0.9 }, { from: 'B', to: 'D', amount: 1.3 }], uncovered: 1.7 });
});
