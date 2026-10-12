import { test, expect } from './test';
import { reconciliationDraftKey } from '../reconciliationDraft';
import { reconciliation } from './finance';

test('failed draft migration retains session fields and warns about persistence', async ({ page }) => {
  const key = reconciliationDraftKey('Visual household', reconciliation.closingDate);
  await page.addInitScript(({ key }) => {
    sessionStorage.setItem(key, JSON.stringify({ 'Household checking': '123.45' }));
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function (name, value) {
      if (this === window.localStorage) throw new DOMException('Full', 'QuotaExceededError');
      return original.call(this, name, value);
    };
  }, { key });
  await page.goto('/reconciliation');
  await expect(page.getByText('Balance drafts could not be stored on this device. Keep a copy before leaving or reloading.')).toBeVisible();
  await expect(page.getByRole('textbox', { name: 'Current balance for Household checking' })).toHaveValue('123.45');
  await expect.poll(() => page.evaluate(key => sessionStorage.getItem(key), key)).toBe(JSON.stringify({ 'Household checking': '123.45' }));
  await page.reload();
  await expect(page.getByRole('textbox', { name: 'Current balance for Household checking' })).toHaveValue('123.45');
});
