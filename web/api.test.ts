import assert from 'node:assert/strict';
import test from 'node:test';
import { getFinanceSnapshot, getTransactions, editTransaction, reconcileAccounts, signOutFromServer, closeBudgetMonth } from './api.ts';
import { finance, reconciliation, transactions } from './visual/finance.ts';

if (!('sessionStorage' in globalThis)) {
  Object.defineProperty(globalThis, 'sessionStorage', { configurable: true, writable: true, value: {} });
  test.after(() => { delete (globalThis as any).sessionStorage; });
}

const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { 'Content-Type': 'application/json' } });
const edit = { transactionId: transactions[0].id, kind: 'expense' as const, date: transactions[0].effectiveDate,
  description: 'Edited receipt', amount: 10, currency: 'EUR' as const, fromAccount: 'Household checking', fromCategory: 'Groceries' };

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

test('out-of-order snapshots cannot replace the account revisions used by edits', async t => {
  const older = deferred<Response>();
  const newer = deferred<Response>();
  let body: any;
  t.mock.method(globalThis, 'fetch', async (path: string, init?: RequestInit) => {
    if (path.includes('asOf=2026-09')) return older.promise;
    if (path.includes('asOf=2026-10')) return newer.promise;
    body = JSON.parse(String(init?.body));
    return json({ updated: true });
  });
  const first = getFinanceSnapshot('2026-09-01');
  const second = getFinanceSnapshot('2026-10-01');
  newer.resolve(json({ ...finance, accounts: finance.accounts.map(item => ({ ...item, ledgerRevision: 20 })) }));
  await second;
  older.resolve(json(finance));
  await first;
  await editTransaction(edit, 1);
  assert.deepEqual(body.expectedAccountRevisions, [{ accountId: 'checking', revision: 20 }]);
});

test('late ledger pages cannot overwrite a refreshed transaction or downgrade its revision', async t => {
  const page = deferred<Response>();
  let body: any;
  t.mock.method(globalThis, 'fetch', async (path: string, init?: RequestInit) => {
    if (path.startsWith('/api/db/finance')) return json(finance);
    if (path.startsWith('/api/db/transactions?')) return page.promise.then(response => response.clone());
    body = JSON.parse(String(init?.body));
    return json({ updated: true });
  });
  await getFinanceSnapshot('2026-10-01');
  const pending = getTransactions({ page: 1 });
  await getFinanceSnapshot('2026-10-01');
  page.resolve(json({ transactions: [{ ...transactions[0], fromAccountId: 'usd', revision: 0 }], total: 1, page: 1, pageSize: 25 }));
  await pending;
  await editTransaction(edit, 1);
  assert.deepEqual(body.expectedAccountRevisions, [{ accountId: 'checking', revision: 12 }]);
  await getTransactions({ page: 1 });
  await editTransaction(edit, 1);
  assert.deepEqual(body.expectedAccountRevisions, [{ accountId: 'checking', revision: 12 }]);
});

for (const storageMode of ['blocked', 'corrupt']) {
  test(`reconciliation retains exact retry payloads with ${storageMode} storage`, async t => {
    const storage = { getItem: () => { if (storageMode === 'blocked') throw Error('blocked'); return '{broken'; },
      setItem: () => { throw Error('full'); }, removeItem: () => { throw Error('blocked'); } };
    t.mock.property(globalThis, 'sessionStorage', storage);
    const bodies: string[] = [];
    t.mock.method(globalThis, 'fetch', async (_path: string, init?: RequestInit) => {
      bodies.push(String(init?.body));
      return bodies.length === 1 ? json({ error: 'Temporary failure' }, 503) : json({ recorded: true });
    });
    const preview: any = { ...reconciliation, accounts: reconciliation.accounts.map(a => ({ ...a, balanceNative: Number(a.balanceNative) })) };
    const payload = (suffix: string) => ({ date: preview.closingDate, accounts: preview.accounts.map((a: any, index: number) => ({ name: a.name, actualBalance: a.balanceNative, transactionId: suffix + index })) });
    await assert.rejects(reconcileAccounts(payload('first'), preview), /Temporary failure/);
    await reconcileAccounts(payload('second'), preview);
    assert.equal(bodies[0], bodies[1]);
  });
}

test('successful logout clears edit state even if storage fails and an old refresh finishes late', async t => {
  const pending = deferred<Response>();
  t.mock.property(globalThis, 'sessionStorage', { removeItem: () => { throw Error('blocked'); } });
  t.mock.method(globalThis, 'fetch', async (path: string) => path === '/api/logout' ? json({ signedOut: true }) : pending.promise);
  const refresh = getFinanceSnapshot('2026-10-01');
  await signOutFromServer();
  pending.resolve(json(finance));
  await assert.rejects(refresh, /session ended/);
  assert.throws(() => editTransaction(edit, 1), /Refresh finance data/);
});

test('month close survives unavailable storage and reuses its operation ID after an ambiguous failure', async t => {
  t.mock.property(globalThis, 'sessionStorage', {
    getItem: () => { throw Error('blocked'); }, setItem: () => { throw Error('full'); }, removeItem: () => { throw Error('blocked'); },
  });
  const bodies: string[] = [];
  t.mock.method(globalThis, 'fetch', async (_path: string, init?: RequestInit) => {
    bodies.push(String(init?.body));
    return bodies.length === 1 ? json({ error: 'Offline' }, 503) : json({ closed: true });
  });
  await assert.rejects(closeBudgetMonth('2026-09'), /Offline/);
  await closeBudgetMonth('2026-09');
  assert.equal(bodies[0], bodies[1]);
});


test('a malformed snapshot does not poison the revisions retained for edits', async t => {
  let malformed = false;
  let body: any;
  t.mock.method(globalThis, 'fetch', async (path: string, init?: RequestInit) => {
    if (path.startsWith('/api/db/finance')) return json(malformed ? {} : finance);
    body = JSON.parse(String(init?.body));
    return json({ updated: true });
  });
  await getFinanceSnapshot('2026-10-01');
  malformed = true;
  await assert.rejects(getFinanceSnapshot('2026-10-01'));
  await editTransaction(edit, 1);
  assert.deepEqual(body.expectedAccountRevisions, [{ accountId: 'checking', revision: 12 }]);
});


test('401 invalidates cached revisions and rejects finance responses already in flight', async t => {
  const pending = deferred<Response>();
  let delayed = false;
  t.mock.method(globalThis, 'fetch', async (path: string) => {
    if (path.startsWith('/api/db/finance')) return delayed ? pending.promise : json(finance);
    return json({ error: 'Session expired' }, 401);
  });
  await getFinanceSnapshot('2026-10-01');
  delayed = true;
  const refresh = getFinanceSnapshot('2026-10-01');
  await assert.rejects(getTransactions({ page: 1 }), /Session expired/);
  pending.resolve(json(finance));
  await assert.rejects(refresh, /session ended/);
  assert.throws(() => editTransaction(edit, 1), /Refresh finance data/);
});

test('temporary server failures retain cached edit state', async t => {
  let body: any;
  t.mock.method(globalThis, 'fetch', async (path: string, init?: RequestInit) => {
    if (path.startsWith('/api/db/finance')) return json(finance);
    if (path.startsWith('/api/db/transactions?')) return json({ error: 'Temporary failure' }, 503);
    body = JSON.parse(String(init?.body));
    return json({ updated: true });
  });
  await getFinanceSnapshot('2026-10-01');
  await assert.rejects(getTransactions({ page: 1 }), /Temporary failure/);
  await editTransaction(edit, 1);
  assert.deepEqual(body.expectedAccountRevisions, [{ accountId: 'checking', revision: 12 }]);
});
