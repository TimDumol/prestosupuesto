import assert from 'node:assert/strict';
import test from 'node:test';
import { readTransactionDraft, transactionDraftKey, writeTransactionDraft } from './transactionDraft.ts';
import type { TransactionDraft } from './transactionDraft.ts';

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return { get length() { return values.size; }, clear: () => values.clear(), key: i => [...values.keys()][i] ?? null, getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); } };
}
const blank: TransactionDraft = { operationId: 'stable-id', baseRevision: '', kind: 'expense', incomeTreatment: 'salary', expectedIncomeMonth: '2026-09', correctsTransactionId: '', date: '2026-09-30', description: '', amount: '', currency: 'EUR', toAmount: '', fromCategory: 'Food', toCategory: '', fromAccount: 'Bank', toAccount: '' };

test('incomplete drafts and retry IDs survive closing/reloading, isolated by user, month, and entry', () => {
  const storage = memoryStorage();
  const key = transactionDraftKey('alice', '2026-09');
  const draft = { ...blank, amount: '1.234,', description: 'Weekly shop' };
  assert.equal(writeTransactionDraft(key, draft, storage), true);
  assert.deepEqual(readTransactionDraft(key, blank, storage), draft);
  for (const other of [transactionDraftKey('bob', '2026-09'), transactionDraftKey('alice', '2026-10'), transactionDraftKey('alice', '2026-09', 'entry')]) assert.deepEqual(readTransactionDraft(other, blank, storage), blank);
  assert.equal(writeTransactionDraft(key, null, storage), true);
  assert.equal(storage.getItem(key), null);
});

test('malformed or blocked storage falls back safely and reports persistence failure', () => {
  const storage = memoryStorage();
  for (const value of ['broken', 'null', '[]', '{}', JSON.stringify({ ...blank, currency: 'INVALID' }), JSON.stringify({ ...blank, amount: 5 })]) {
    storage.setItem('draft', value);
    assert.deepEqual(readTransactionDraft('draft', blank, storage), blank);
  }
  storage.getItem = () => { throw Error('blocked'); };
  storage.setItem = () => { throw Error('full'); };
  storage.removeItem = () => { throw Error('blocked'); };
  assert.deepEqual(readTransactionDraft('draft', blank, storage), blank);
  assert.equal(writeTransactionDraft('draft', blank, storage), false);
  assert.equal(writeTransactionDraft('draft', null, storage), false);
});
