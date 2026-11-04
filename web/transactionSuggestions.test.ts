import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyDescriptionSuggestion, descriptionSuggestions, mostUsedAccount, suggestedCategory } from './transactionSuggestions.ts';
import type { FinanceSnapshot, GatewayTransaction } from './contracts.ts';
import type { TransactionDraft } from './transactionDraft.ts';
const row = (description: string, date: string, account = 'Daily'): GatewayTransaction => ({ description, date, kind: 'expense', fromCategory: 'Groceries', fromAccount: account, transactionId: date, row: 1 } as GatewayTransaction);
const rows = [row('Café corner market', '2026-10-01'), row('Café corner market', '2026-10-02'), row('Monthly rent', '2026-10-03', 'Rent')];
const snapshot = { accounts: [{ name: 'Rent', currency: 'USD' }, { name: 'Daily', currency: 'EUR' }], budgets: [{ name: 'Unallocated' }, { name: 'Groceries' }], recentTransactions: rows } as FinanceSnapshot;
test('autocomplete normalizes accents, supports reordered words and typos, deduplicates and excludes edits', () => {
  assert.equal(descriptionSuggestions('market cafe', rows, 'expense')[0].date, '2026-10-02');
  assert.equal(descriptionSuggestions('conrer', rows, 'expense').length, 1);
  assert.equal(descriptionSuggestions('market', rows, 'income').length, 0);
  assert.equal(descriptionSuggestions('market', rows, 'expense', '2026-10-02')[0].date, '2026-10-01');
});
test('defaults use category history and category keywords', () => {
  assert.equal(mostUsedAccount(snapshot)?.name, 'Daily');
  assert.equal(mostUsedAccount(snapshot, 'Groceries')?.name, 'Daily');
  assert.equal(suggestedCategory('supermarket shopping', snapshot), 'Groceries');
});
test('explicit autocomplete fills valid links while preserving amount, date and identity', () => {
  const draft = { kind: 'expense', amount: '19.75', date: '2026-11-01', operationId: 'new-id', fromAccount: 'Rent', fromCategory: '', currency: 'USD' } as TransactionDraft;
  const next = applyDescriptionSuggestion(draft, rows[0], snapshot);
  assert.equal(next.fromAccount, 'Daily');
  assert.equal(next.fromCategory, 'Groceries');
  assert.equal(next.currency, 'EUR');
  assert.equal(next.amount, draft.amount);
  assert.equal(next.date, draft.date);
  assert.equal(next.operationId, draft.operationId);
});
