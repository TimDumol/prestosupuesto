import assert from 'node:assert/strict';
import test from 'node:test';
import { formatTransactionDate, localDateIso } from './date.ts';
import { parseQuickEntry, resolveAccountQuery, suggestCategory } from './entryIntelligence.ts';

const accounts = [
  { id: 'H Seabank', name: 'H Seabank', currency: 'PHP' as const, kind: 'bank' as const },
  { id: 'H GoTyme', name: 'H GoTyme', currency: 'PHP' as const, kind: 'bank' as const },
];

const categories = [
  { id: 'home', name: 'Household', monthly: 0, rollover: 0, adjustment: 0, spent: 0 },
  { id: 'food', name: 'Food & groceries', monthly: 0, rollover: 0, adjustment: 0, spent: 0 },
];

test('parses the household PHP quick-entry format', () => {
  assert.deepEqual(parseQuickEntry('PHP 2,287.43 / h seabank / cleaner for aug 30'), {
    amount: 2287.43,
    currency: 'PHP',
    accountQuery: 'h seabank',
    description: 'cleaner for aug 30',
  });
});

test('accepts European separators and account alias', () => {
  assert.equal(parseQuickEntry('EUR 2.287,43 / H GoTyme / test')?.amount, 2287.43);
  assert.equal(resolveAccountQuery('th gotyme', accounts)?.id, 'H GoTyme');
});

test('suggests a category from keywords and prior descriptions', () => {
  assert.equal(suggestCategory('Cleaner for Aug 30', categories, [])?.categoryId, 'home');
  assert.equal(suggestCategory('Weekend provisions', categories, [{
    id: 'old',
    kind: 'expense',
    description: 'Weekend provisions',
    amount: 10,
    currency: 'EUR',
    date: '2026-08-20',
    dateLabel: '20 Aug 2026',
    categoryFrom: 'food',
  }])?.categoryId, 'food');
});

test('formats today and yesterday with the actual date', () => {
  const now = new Date(2026, 7, 24, 12);
  assert.equal(localDateIso(now), '2026-08-24');
  assert.match(formatTransactionDate('2026-08-24', now), /^Today · /);
  assert.match(formatTransactionDate('2026-08-23', now), /^Yesterday · /);
  assert.match(formatTransactionDate('2026-08-22', now), /2026/);
});
