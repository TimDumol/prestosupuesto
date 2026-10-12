import { test, expect } from './test';
import { finance, transactions } from './finance';

test('shareable ledger filters survive reload and phone tables stay within viewport', async ({ page }) => {
  await page.goto('/transactions?month=2026-10&account=Household+checking&search=groceries&history=all');
  await expect(page.getByText('Weekly groceries · neighbourhood market', { exact: true })).toBeVisible();
  await expect(page.getByText('Dinner with friends', { exact: true })).toHaveCount(0);
  await page.reload();
  await expect(page.getByText('Weekly groceries · neighbourhood market', { exact: true })).toBeVisible();
  expect(new URL(page.url()).searchParams.get('history')).toBe('all');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('budget proposal previews before recording and confirms resulting balances', async ({ page }) => {
  let writes = 0;
  await page.route('**/api/db/transactions', async route => { writes++; await route.fulfill({ json: { duplicate: false } }); });
  await page.goto('/budgets');
  await page.getByText('Review a balancing proposal using available categories').click();
  await page.getByRole('button', { name: 'Preview this move' }).first().click();
  expect(writes).toBe(0);
  await expect(page.getByRole('status').filter({ hasText: '→' })).toBeVisible();
  await page.getByRole('button', { name: 'Record budget move', exact: true }).click();
  await expect(page.getByText(/Budget move saved/)).toBeVisible();
  expect(writes).toBe(1);
});

test('duplicate warning permits legitimate separate household purchases', async ({ page }) => {
  let writes = 0;
  await page.route('**/api/db/transactions', async route => { writes++; await route.fulfill({ json: { duplicate: false } }); });
  await page.goto('/transactions');
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  const form = page.locator('.transaction-editor');
  await form.getByLabel('Description', { exact: true }).fill('Weekly groceries neighbourhood market');
  await form.getByLabel('Amount', { exact: true }).fill('87.35');
  await form.getByLabel('Budget category', { exact: true }).selectOption('Groceries');
  await form.getByLabel('Account', { exact: true }).selectOption('Household checking');
  await form.getByLabel('Transaction date · October 2026', { exact: true }).fill('2026-10-17');
  page.once('dialog', async dialog => { expect(dialog.message()).toContain('Possible duplicate'); await dialog.dismiss(); });
  await form.getByRole('button', { name: 'Add transaction', exact: true }).click();
  await expect(form.getByRole('button', { name: 'Add transaction', exact: true })).toBeEnabled();
  expect(writes).toBe(0);
  page.once('dialog', async dialog => dialog.accept());
  await form.getByRole('button', { name: 'Add transaction', exact: true }).click();
  await expect(page.getByText('Transaction saved.', { exact: true })).toBeVisible();
  expect(writes).toBe(1);
});

test('schedule edit touches only the selected category and cancellation restores prior plan', async ({ page }) => {
  await page.route('**/api/db/finance?*', route => route.fulfill({ json: { ...finance, scheduledAllocations: [{ effectiveStartMonth: '2026-11', name: 'Groceries', amountEur: '500.00' }] } }));
  let payload: any;
  await page.route('**/api/db/finance/monthly-budget', async route => { payload = route.request().postDataJSON(); await route.fulfill({ json: { saved: true } }); });
  await page.goto('/budgets');
  await page.getByRole('button', { name: 'Edit', exact: true }).click();
  await page.getByLabel('Groceries monthly allocation').fill('550');
  await page.getByRole('button', { name: 'Save monthly budget' }).click();
  await expect(page.getByText('Monthly budget saved.', { exact: true })).toBeVisible();
  expect(payload.categories).toEqual([{ name: 'Groceries', amount: '550' }]);
  await page.route('**/api/db/finance/monthly-budget/cancel', route => route.fulfill({ json: { cancelled: true } }));
  page.once('dialog', dialog => dialog.accept());
  await page.getByRole('button', { name: 'Cancel change' }).click();
  await expect(page.getByText('Scheduled change cancelled.', { exact: true })).toBeVisible();
});

test('discrepancy notes survive reopening and history opens the affected account', async ({ page }) => {
  await page.route('**/api/db/finance?*', route => route.fulfill({ json: { ...finance, asOf: '2026-09-01', budgetMonth: '2026-09' } }));
  await page.goto('/reconciliation');
  await page.getByLabel('Current balance for Household checking').fill('2800');
  await page.getByLabel('Investigation notes (on this device)').fill('Check bank fee');
  await page.reload();
  await expect(page.getByLabel('Investigation notes (on this device)')).toHaveValue('Check bank fee');
  await page.getByRole('button', { name: 'Inspect Household checking transactions' }).click();
  await expect(page.getByText('Weekly groceries · neighbourhood market', { exact: true })).toBeVisible();
  expect(new URL(page.url()).searchParams.get('account')).toBe('Household checking');
});

test('household history identifies who edited each version', async ({ page }) => {
  await page.route('**/api/db/transactions/history?*', route => route.fulfill({ json: { history: [
    { revision: 1, at: '2026-10-17T10:00:00Z', by: 'Alex', details: { transaction: { description: 'Market purchase', amount: '87.35', inputCurrency: 'EUR' } } },
    { revision: 2, at: '2026-10-17T11:00:00Z', by: 'Sam', details: { request: { description: 'Weekly groceries', amount: '89.35', inputCurrency: 'EUR' } } },
  ] } }));
  await page.goto('/transactions?search=groceries');
  await page.getByText('Edit history · version 1', { exact: true }).click();
  await expect(page.getByText(/Version 1 · Alex/)).toBeVisible();
  await expect(page.getByText(/Version 2 · Sam/)).toBeVisible();
  await expect(page.getByText(/89.35 EUR/)).toBeVisible();
});

test('successful write followed by refresh failure is reported as saved and stale', async ({ page }) => {
  await page.route('**/api/db/transactions', async route => {
    await page.route('**/api/db/finance?*', next => next.fulfill({ status: 503, json: { error: 'Offline' } }));
    await route.fulfill({ json: { duplicate: false } });
  });
  await page.goto('/budgets');
  await page.getByRole('button', { name: 'Record budget move', exact: true }).click();
  await expect(page.getByText(/Budget move saved/)).toContainText('Refresh failed');
  await expect(page.getByText(/May be out of date/)).toBeVisible();
});


test('edit conflicts keep draft fields and offer the latest household entry for review', async ({ page }) => {
  let conflicted = false;
  await page.route('**/api/db/transactions?*', async route => {
    if (!conflicted) return route.fallback();
    await route.fulfill({ json: { transactions: [{ ...transactions[0], revision: 2, description: 'Partner updated receipt' }], total: 1, page: 1, pageSize: 100 } });
  });
  await page.route('**/api/db/transactions/edit', async route => {
    conflicted = true;
    await route.fulfill({ status: 409, json: { error: 'This transaction was changed by another household member.' } });
  });
  await page.goto('/transactions?search=groceries');
  await page.getByRole('button', { name: 'Edit', exact: true }).click();
  await page.locator('.transaction-editor').getByLabel('Description', { exact: true }).fill('My receipt draft');
  await page.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(page.getByText(/Another household member changed this entry/)).toBeVisible();
  await expect(page.locator('.transaction-editor').getByLabel('Description', { exact: true })).toHaveValue('My receipt draft');
  page.once('dialog', dialog => dialog.accept());
  await page.getByRole('button', { name: 'Use latest saved entry' }).click();
  await expect(page.locator('.transaction-editor').getByLabel('Description', { exact: true })).toHaveValue('Partner updated receipt');
});


test('desktop ledger fits its container with long descriptions and expanded history', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop');
  await page.route('**/api/db/transactions?*', route => route.fulfill({ json: {
    transactions: [{ ...transactions[0], description: 'A very long transaction description '.repeat(12), createdBy: 'household-member-with-a-long-name@example.com' }], total: 1, page: 1, pageSize: 25,
  } }));
  await page.route('**/api/db/transactions/history?*', route => route.fulfill({ json: { history: [
    { revision: 1, at: '2026-10-17T10:00:00Z', by: 'Alex', details: { transaction: { description: 'Original purchase', amount: '87.35', inputCurrency: 'EUR' } } },
  ] } }));
  for (const width of [1440, 1100]) {
    await page.setViewportSize({ width, height: 1000 });
    await page.goto('/transactions');
    await page.getByText('Edit history · version 1', { exact: true }).click();
    await expect(page.getByText('Original purchase', { exact: true })).toBeVisible();
    const container = page.locator('.ledger-table .ant-table-content');
    expect(await container.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
});
