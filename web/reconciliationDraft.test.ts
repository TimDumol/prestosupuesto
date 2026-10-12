import assert from 'node:assert/strict';
import test from 'node:test';
import { readReconciliationDraft, reconciliationDraftKey, writeReconciliationDraft } from './reconciliationDraft.ts';

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() { return values.size; },
    clear: () => values.clear(),
    key: (index) => [...values.keys()][index] ?? null,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => { values.set(key, value); },
    removeItem: (key) => { values.delete(key); },
  };
}

test('drafts retain incomplete values across reloads and isolate users and closing months', () => {
  const storage = memoryStorage();
  const key = reconciliationDraftKey('alice', '2026-09-30');
  const balances = { Bank: '123.', Wallet: '-', Cash: '' };
  assert.equal(writeReconciliationDraft(storage, key, balances), true);
  assert.deepEqual(readReconciliationDraft(storage, key), balances);
  assert.deepEqual(readReconciliationDraft(storage, reconciliationDraftKey('bob', '2026-09-30')), {});
  assert.deepEqual(readReconciliationDraft(storage, reconciliationDraftKey('alice', '2026-08-31')), {});
  writeReconciliationDraft(storage, key, {});
  assert.equal(storage.getItem(key), null);
});

test('malformed drafts and unavailable storage do not break the form', () => {
  const storage = memoryStorage();
  for (const value of ['invalid', 'null', '[]', '42']) {
    storage.setItem('draft', value);
    assert.deepEqual(readReconciliationDraft(storage, 'draft'), {});
  }
  storage.setItem('draft', JSON.stringify({ Bank: '0', Wallet: 123 }));
  assert.deepEqual(readReconciliationDraft(storage, 'draft'), { Bank: '0' });
  storage.getItem = () => { throw new Error('blocked'); };
  storage.setItem = () => { throw new Error('full'); };
  storage.removeItem = () => { throw new Error('blocked'); };
  assert.deepEqual(readReconciliationDraft(storage, 'draft'), {});
  assert.equal(writeReconciliationDraft(storage, 'draft', { Bank: '1' }), false);
  assert.equal(writeReconciliationDraft(storage, 'draft', {}), false);
});
