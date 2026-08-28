import assert from 'node:assert/strict';
import test from 'node:test';
import type { Transaction } from './domain.ts';
import { mergeTransactions } from './transactionCache.ts';

function transaction(overrides: Partial<Transaction>): Transaction {
  return {
    id: 'transaction',
    kind: 'expense',
    description: 'Test',
    amount: 10,
    currency: 'EUR',
    date: '2026-08-25',
    dateLabel: 'Today',
    ...overrides,
  };
}

test('remote rows replace cached rows while older history remains available', () => {
  const cached = [
    transaction({ id: 'cached-current', sourceRow: 10 }),
    transaction({ id: 'historical', sourceRow: 2, date: '2026-06-01' }),
  ];
  const remote = [transaction({ id: 'remote-current', sourceRow: 10, amount: 12 })];

  assert.deepEqual(mergeTransactions(cached, remote).map((item) => item.id), [
    'remote-current',
    'historical',
  ]);
});

test('optimistic transactions stay ahead of refreshed remote history', () => {
  const pending = transaction({ id: 'pending', syncStatus: 'pending' });
  const remote = transaction({ id: 'remote', sourceRow: 10 });
  assert.deepEqual(mergeTransactions([pending], [remote]).map((item) => item.id), ['pending', 'remote']);
});
