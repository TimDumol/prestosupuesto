import assert from 'node:assert/strict';
import test from 'node:test';
import {
  assignments,
  columnName,
  extractSpreadsheetId,
  parseArgs,
  quoteSheetTitle,
  resolveRecord,
} from './lib.mjs';

test('extractSpreadsheetId accepts URLs and raw IDs', () => {
  assert.equal(
    extractSpreadsheetId('https://docs.google.com/spreadsheets/d/abc_123-xy/edit#gid=0'),
    'abc_123-xy',
  );
  assert.equal(extractSpreadsheetId('abc_123-xy'), 'abc_123-xy');
});

test('A1 helpers handle quoting and columns beyond Z', () => {
  assert.equal(quoteSheetTitle("August's data"), "'August''s data'");
  assert.equal(columnName(1), 'A');
  assert.equal(columnName(26), 'Z');
  assert.equal(columnName(27), 'AA');
  assert.equal(columnName(703), 'AAA');
});

test('arguments preserve repeated flags', () => {
  assert.deepEqual(parseArgs(['--sheet', 'Transactions', '--set', 'Amount=12.5', '--set=Currency=EUR']), {
    _: [],
    sheet: 'Transactions',
    set: ['Amount=12.5', 'Currency=EUR'],
  });
});

test('assignments parse simple scalar values', () => {
  assert.deepEqual(assignments(['Amount=12.5', 'Cleared=true', 'Description=Coffee']), {
    Amount: 12.5,
    Cleared: true,
    Description: 'Coffee',
  });
});

test('records map to headers independent of case and punctuation', () => {
  const resolved = resolveRecord(
    ['Date', 'From category', 'To Currency'],
    { date: '2026-08-23', from_category: 'Food', 'to-currency': 'EUR' },
  );
  assert.equal(resolved.get(0).value, '2026-08-23');
  assert.equal(resolved.get(1).value, 'Food');
  assert.equal(resolved.get(2).value, 'EUR');
});

test('records reject fields absent from the sheet header', () => {
  assert.throws(() => resolveRecord(['Date'], { Description: 'Coffee' }), /not found/);
});
