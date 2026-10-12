import { test } from 'node:test';
import assert from 'node:assert/strict';
import { yearlyFeePlan } from './yearlyFee.ts';

test('yearly fees round up to cents and spread the ongoing fee over twelve months', () => {
  assert.equal(yearlyFeePlan(100, 80, '2026-10')?.monthly, 8.34);
  assert.equal(yearlyFeePlan(120, 0, '2026-10')?.monthly, 10);
});
test('next payment uses saved funds and includes this month across year boundaries', () => {
  assert.deepEqual(yearlyFeePlan(120, 30, '2026-10', '2027-01'), {
    months: 4, monthly: 22.5, needed: 90, saved: 30, annualMonthly: 10,
  });
  assert.equal(yearlyFeePlan(120, -20, '2026-10', '2026-10')?.monthly, 140);
  assert.equal(yearlyFeePlan(120, 150, '2026-10', '2026-10')?.monthly, 0);
});
test('past or malformed payment months and invalid fees are rejected', () => {
  for (const month of ['2026-09', '2026-13', 'invalid']) assert.equal(yearlyFeePlan(120, 0, '2026-10', month), null);
  for (const fee of [0, -1, NaN, Infinity]) assert.equal(yearlyFeePlan(fee, 0, '2026-10'), null);
});
