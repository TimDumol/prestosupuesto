import { test, expect } from './test';

const pages = [
  ['overview', '/', 'October 2026 at a glance'],
  ['budgets', '/budgets', 'Balance the month'],
  ['history', '/history', 'Account history'],
  ['reconciliation', '/reconciliation', 'Reconcile your accounts'],
  ['transactions', '/transactions', 'Transactions'],
] as const;

for (const [name, path, heading] of pages) {
  test(`${name} authenticated`, async ({ page }) => {
    await page.goto(path);
    await expect(page.getByRole('heading', { name: heading, exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Continue with Pocket ID' })).toHaveCount(0);
    await expect(page.getByRole('alert')).toHaveCount(0);
    if (name === 'transactions') await expect(page.getByText('Weekly groceries · neighbourhood market', { exact: true })).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    await expect(page).toHaveScreenshot(`${name}.png`, { fullPage: true });
  });
}

test('transaction entry with realistic account and category choices', async ({ page }) => {
  await page.goto('/transactions');
  await expect(page.getByText('Weekly groceries · neighbourhood market', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  await expect(page).toHaveScreenshot('transaction-entry.png', { fullPage: true });
});

test('signed out users see the login screen', async ({ page }) => {
  let financeRequests = 0;
  page.on('request', request => { if (request.url().includes('/api/db/')) financeRequests++; });
  await page.route('**/api/session', route => route.fulfill({ json: {
    authenticated: false, email: null, pocketAuthenticated: false, pocketUsername: null,
  } }));
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Continue with Pocket ID' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'October 2026 at a glance' })).toHaveCount(0);
  await expect(page).toHaveScreenshot('signed-out.png', { fullPage: true });
  expect(financeRequests).toBe(0);
});
