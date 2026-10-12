import { test, expect } from './test';
import { finance } from './finance';

test('phone navigation labels and funding details remain accessible', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'October 2026 at a glance' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Transactions', exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  const details = page.locator('.funding-details');
  await expect(details).not.toHaveAttribute('open', '');
  await details.getByText('Funding details and calculations').click();
  await expect(details).toHaveAttribute('open', '');
  await expect(details.getByText('Income still expected', { exact: true })).toBeVisible();
});

test('localized amounts and optional correction drafts survive navigation and reload', async ({ page }) => {
  await page.goto('/transactions');
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  const form = page.locator('.transaction-editor');
  await form.getByLabel('Description', { exact: true }).fill('Receipt to finish later');
  await form.getByLabel('Amount', { exact: true }).fill('1.234,56');
  await expect(form.getByText('Interpreted as 1,234.56', { exact: true })).toBeVisible();
  await expect(form.locator('.transaction-preview')).toContainText('decreases by €1,234.56');
  await expect(form.getByLabel('Correction for earlier transaction (optional)')).toBeHidden();
  await form.getByText('More details', { exact: true }).click();
  await expect(form.getByLabel('Correction for earlier transaction (optional)')).toBeVisible();
  await page.getByRole('button', { name: 'Budgets', exact: true }).click();
  await page.getByRole('button', { name: 'Transactions', exact: true }).click();
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  await expect(form.getByLabel('Amount', { exact: true })).toHaveValue('1.234,56');
  await page.reload();
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  await expect(form.getByLabel('Description', { exact: true })).toHaveValue('Receipt to finish later');
  await expect(form.getByLabel('Amount', { exact: true })).toHaveValue('1.234,56');
});

test('save and add another clears receipt fields and gives each payment a new identity', async ({ page }) => {
  const submitted: Record<string, unknown>[] = [];
  await page.route('**/api/db/transactions', async route => {
    if (route.request().method() !== 'POST') return route.fallback();
    submitted.push(route.request().postDataJSON());
    await route.fulfill({ json: { duplicate: false, transaction: {} } });
  });
  await page.goto('/transactions');
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  const form = page.locator('.transaction-editor');
  await form.getByLabel('Description', { exact: true }).fill('First receipt');
  await form.getByLabel('Budget category', { exact: true }).selectOption({ label: 'Groceries' });
  await form.getByLabel('Amount', { exact: true }).fill('=10 + 2,50');
  const account = await form.getByLabel('Account', { exact: true }).inputValue();
  await form.getByRole('button', { name: 'Save and add another', exact: true }).click();
  await expect(form.getByLabel('Description', { exact: true })).toHaveValue('');
  await expect(form.getByLabel('Amount', { exact: true })).toHaveValue('');
  await expect(form.getByLabel('Budget category', { exact: true })).toHaveValue('Groceries');
  await expect(form.getByLabel('Account', { exact: true })).toHaveValue(account);
  await form.getByLabel('Description', { exact: true }).fill('Second receipt');
  await form.getByLabel('Amount', { exact: true }).fill('3,75');
  await form.getByRole('button', { name: 'Add transaction', exact: true }).click();
  await expect(form).toHaveCount(0);
  expect(submitted).toHaveLength(2);
  expect(submitted[0].amount).toBe('12.5');
  expect(submitted[1].amount).toBe('3.75');
  expect(submitted[0].transactionId).not.toBe(submitted[1].transactionId);
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  await expect(form.getByLabel('Description', { exact: true })).toHaveValue('');
});

test('cross-currency transfer previews both amounts with no category movement', async ({ page }) => {
  await page.goto('/transactions');
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  const form = page.locator('.transaction-editor');
  await form.getByLabel('Type', { exact: true }).selectOption('transfer');
  await form.getByLabel('To account', { exact: true }).selectOption({ label: 'USD travel wallet · USD' });
  await form.getByLabel('Amount', { exact: true }).fill('100');
  await form.getByLabel('Amount received · USD', { exact: true }).fill('105');
  const preview = form.locator('.transaction-preview');
  await expect(preview).toContainText('Household checking decreases by €100.00');
  await expect(preview).toContainText('USD travel wallet increases by US$105.00');
  await expect(preview).toContainText('Category budgets stay unchanged');
});

test('salary preview does not guess settlement values for an expectation in another currency', async ({ page }) => {
  await page.route('**/api/db/finance?*', route => route.fulfill({ json: {
    ...finance, budgetCoverage: { ...finance.budgetCoverage,
      expectations: finance.budgetCoverage.expectations.map(item => ({ ...item, currency: 'USD' })),
    },
  } }));
  await page.goto('/transactions');
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  const form = page.locator('.transaction-editor');
  await form.getByLabel('Type', { exact: true }).selectOption('income');
  await form.getByLabel('Amount', { exact: true }).fill('100');
  const preview = form.locator('.transaction-preview');
  await expect(preview).toContainText('Category allocations stay unchanged');
  await expect(preview).toContainText('Settles the selected month’s expected salary');
  await expect(preview).not.toContainText('Expected salary decreases by');
});
