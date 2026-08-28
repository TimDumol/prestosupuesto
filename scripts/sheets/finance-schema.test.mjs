import assert from 'node:assert/strict';
import test from 'node:test';
import { buildTransaction, sheetsSerial, uuidV7 } from './finance-schema.mjs';

test('creates time-ordered UUIDv7 identifiers with the correct version and variant', () => {
  const first = uuidV7(1_700_000_000_000, new Uint8Array(16));
  const second = uuidV7(1_700_000_000_001, new Uint8Array(16));
  assert.match(first, /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  assert.ok(first < second);
});

test('converts an ISO date to the workbook serial date', () => {
  assert.equal(sheetsSerial('2026-08-23'), 46257);
});

test('maps an expense to category and account sinks', () => {
  const transaction = buildTransaction({
    kind: 'expense', date: '2026-08-23', amount: 1, description: 'Test',
    fromCategory: 'Groceries', fromAccount: 'Checking EUR', testId: '12345678-test',
  });
  assert.equal(transaction.values.get(5), 'Groceries');
  assert.equal(transaction.values.get(6), 'Expense');
  assert.equal(transaction.values.get(7), 'Checking EUR');
  assert.equal(transaction.values.get(8), 'Expense');
});

test('maps income from the system source to a budget and account', () => {
  const transaction = buildTransaction({
    kind: 'income', date: '2026-08-23', amount: 2, toCategory: 'Budget A',
    toAccount: 'Wallet USD', testId: '12345678-test',
  });
  assert.equal(transaction.values.get(5), 'Income');
  assert.equal(transaction.values.get(6), 'Budget A');
  assert.equal(transaction.values.get(7), 'Income');
  assert.equal(transaction.values.get(8), 'Wallet USD');
});

test('maps account and budget transfers to their neutral system values', () => {
  const transfer = buildTransaction({
    kind: 'transfer', date: '2026-08-23', amount: 3,
    fromAccount: 'Checking EUR', toAccount: 'Savings EUR', testId: '12345678-test',
  });
  assert.equal(transfer.values.get(5), 'Balance Transfer');
  assert.equal(transfer.values.get(6), 'Balance Transfer');

  const reallocation = buildTransaction({
    kind: 'reallocate', date: '2026-08-23', amount: 4,
    fromCategory: 'Budget A', toCategory: 'Budget B', testId: '12345678-test',
  });
  assert.equal(reallocation.values.get(7), 'Reallocation');
  assert.equal(reallocation.values.get(8), 'Reallocation');
});

test('cross-currency transfers can replace only the destination amount formula', () => {
  const transaction = buildTransaction({
    kind: 'transfer', date: '2026-08-23', amount: 5, toAmount: 4,
    fromAccount: 'Wallet USD', toAccount: 'Checking EUR', testId: '12345678-test',
  });
  assert.equal(transaction.values.get(4), 4);
});

test('a rare transaction-currency override replaces only the currency formula', () => {
  const transaction = buildTransaction({
    kind: 'expense', date: '2026-08-23', amount: 5, currency: 'USD',
    fromCategory: 'Groceries', fromAccount: 'Checking EUR', testId: '12345678-test',
  });
  assert.equal(transaction.values.get(3), 'USD');
  assert.equal(transaction.values.get(4), undefined);
});
