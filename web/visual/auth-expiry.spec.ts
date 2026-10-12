import { test, expect } from './test';

test('a 401 hides cached finance data and retains unfinished transaction drafts', async ({ page }) => {
  await page.goto('/transactions');
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  await page.locator('form.transaction-editor').getByLabel('Description', { exact: true }).fill('Unfinished auth regression');
  await expect.poll(() => page.evaluate(() => Object.values(localStorage).some(value => value.includes('Unfinished auth regression')))).toBe(true);
  await page.route('**/api/db/finance?**', route => route.fulfill({ status: 401, json: { error: 'Sign in again.' } }));
  await page.getByRole('button', { name: '↻ Refresh', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Continue with Pocket ID' })).toBeVisible();
  await expect(page.getByText('Weekly groceries · neighbourhood market', { exact: true })).not.toBeVisible();
  await expect.poll(() => page.evaluate(() => Object.values(localStorage).some(value => value.includes('Unfinished auth regression')))).toBe(true);
});

test('returning to the tab detects invalidation, but a network failure retains authentication', async ({ page }) => {
  await page.goto('/transactions');
  await expect(page.getByText('Weekly groceries · neighbourhood market', { exact: true })).toBeVisible();
  let expired = false;
  await page.route('**/api/session', route => route.fulfill(expired
    ? { json: { pocketAuthenticated: false, authenticated: false } }
    : { status: 503, json: { error: 'Temporary failure' } }));
  const offline = page.waitForResponse(response => response.url().endsWith('/api/session'));
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await offline;
  await expect(page.getByText('Weekly groceries · neighbourhood market', { exact: true })).toBeVisible();
  expired = true;
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await expect(page.getByRole('button', { name: 'Continue with Pocket ID' })).toBeVisible();
});
