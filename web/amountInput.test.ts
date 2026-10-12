import assert from 'node:assert/strict';
import test from 'node:test';
import { parseAmount, parseMoneyAmount } from './amountInput.ts';

test('bank balances in European, US, and space-grouped formats preserve their value', () => {
  for (const text of ['1234.56', '1234,56', '1.234,56', '1,234.56', '1 234,56', '1\u00a0234.56', '€ 1.234,56', '1,234.56 USD']) {
    assert.equal(parseMoneyAmount(text), 1234.56, text);
  }
  assert.equal(parseMoneyAmount('-1.234,56'), -1234.56);
  assert.equal(parseMoneyAmount('1.234.567,89'), 1234567.89);
  assert.equal(parseMoneyAmount('1,234,567.89'), 1234567.89);
  assert.equal(parseMoneyAmount('1.234.567'), 1234567);
  assert.equal(parseMoneyAmount('0'), 0);
});

test('malformed and overprecise money is rejected rather than changed silently', () => {
  for (const text of ['', ' ', '-', '1.23,45', '1,23,456', '12 34,56', '1.234,5,6', '1e3', 'Infinity', '1.234', '1,234', '9007199254740991']) assert.equal(parseMoneyAmount(text), null, text);
  assert.equal(parseAmount('1.234'), 1.234, 'single separator is decimal');
  assert.equal(parseMoneyAmount(',50'), 0.5);
});

test('formulas retain precedence, signs, and localized literals without evaluating arbitrary code', () => {
  assert.equal(parseMoneyAmount('=(1.234,56 + 10,44) / 5'), 249);
  assert.equal(parseMoneyAmount('=123+382'), 505);
  assert.equal(parseMoneyAmount('=2×(3+4)÷2'), 7);
  assert.equal(parseMoneyAmount('=0.1+0.2'), 0.3);
  for (const text of ['=1/0', '=alert(1)', '=1.23,45+2', '=1/3', '=2+(3', '=' + '1'.repeat(257)]) assert.equal(parseMoneyAmount(text), null, text);
});
