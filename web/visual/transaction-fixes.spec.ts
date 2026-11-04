import { test, expect } from './test';

for (const editing of [false, true]) {
  test(`budget move ${editing ? 'edit' : 'create'} sends only category IDs and EUR amount`, async ({ page }) => {
    let saved: Record<string, unknown> | undefined;
    const endpoint = editing ? '/api/db/transactions/edit' : '/api/db/transactions';
    await page.route('**' + endpoint, async route => {
      if (route.request().method() !== 'POST') return route.fallback();
      saved = route.request().postDataJSON();
      await route.fulfill({ json: { updated: true, duplicate: false } });
    });
    await page.goto('/transactions');
    await expect(page.getByText('Weekly groceries · neighbourhood market', { exact: true })).toBeVisible();
    if (editing) await page.getByRole('button', { name: 'Edit', exact: true }).first().click();
    else await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
    await page.locator('form.transaction-editor').getByLabel('Type', { exact: true }).selectOption('reallocate');
    await page.locator('form.transaction-editor').getByLabel('Description', { exact: true }).fill('move leftover rent allocation');
    await page.locator('form.transaction-editor').getByLabel('Move from', { exact: true }).click();
    await page.getByRole('option', { name: 'Housing', exact: true }).filter({ visible: true }).last().click();
    await page.locator('form.transaction-editor').getByLabel('Move to', { exact: true }).click();
    await page.getByRole('option', { name: 'Travel fund', exact: true }).filter({ visible: true }).last().click();
    await page.locator('form.transaction-editor').getByLabel('Amount', { exact: true }).fill('1400');
    await page.getByRole('button', { name: editing ? 'Save changes' : 'Add transaction', exact: true }).click();
    await expect(page.getByText('Transaction saved.', { exact: true })).toBeVisible();
    expect(saved).toMatchObject({ kind: 'reallocate', amount: '1400', description: 'move leftover rent allocation', fromCategoryId: 'category-0', toCategoryId: 'category-5' });
    for (const field of ['currency', 'toAmount', 'fromAccountId', 'toAccountId']) expect(saved).not.toHaveProperty(field);
  });
}

test('new transaction failure preserves fields and gives retry advice', async ({ page }) => {
  await page.route('**/api/db/transactions', async route => {
    if (route.request().method() !== 'POST') return route.fallback();
    await route.fulfill({ status: 400, json: { error: 'Example validation error.' } });
  });
  await page.goto('/transactions');
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  await page.locator('form.transaction-editor').getByLabel('Description', { exact: true }).fill('Test expense');
  await page.locator('form.transaction-editor').getByLabel('Amount', { exact: true }).fill('12');
  await page.getByRole('button', { name: 'Add transaction', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('Review the fields and try again.');
  await expect(page.getByRole('alert')).not.toContainText('reopen the latest entry');
  await expect(page.locator('form.transaction-editor').getByLabel('Description', { exact: true })).toHaveValue('Test expense');
});

for (const occurredAt of ['2026-10-17T08:15:00Z', null]) {
  test(`editing preserves ${occurredAt ? 'recorded UTC time' : 'date-only entries'}`, async ({ page }) => {
    const { transactions } = await import('./finance');
    await page.route('**/api/db/transactions?*', route => route.fulfill({ json: {
      transactions: [{ ...transactions[0], occurredAt, effectiveDate: occurredAt ? '2026-10-17' : '2026-10-18' }], total: 1, page: 1, pageSize: 50,
    } }));
    let saved: any;
    await page.route('**/api/db/transactions/edit', async route => {
      saved = route.request().postDataJSON();
      await route.fulfill({ json: { updated: true, revision: 2 } });
    });
    await page.goto('/transactions');
    await page.getByRole('button', { name: 'Edit', exact: true }).click();
    await page.locator('.transaction-editor').getByLabel('Description', { exact: true }).fill('Receipt description corrected');
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(page.getByText('Transaction saved.', { exact: true })).toBeVisible();
    expect(saved.occurredAt).toBe(occurredAt);
    expect(saved.dateOnly).toBe(!occurredAt);
  });
}

test('failed logout retains authenticated data and reports the error', async ({ page }) => {
  await page.route('**/api/logout', route => route.fulfill({ status: 503, json: { error: 'Sign-out temporarily unavailable' } }));
  await page.goto('/transactions');
  await expect(page.getByText('Weekly groceries · neighbourhood market', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('Sign-out temporarily unavailable');
  await expect(page.getByText('Weekly groceries · neighbourhood market', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Continue with Pocket ID' })).toHaveCount(0);
});

test('save another immediately persists its selected accounts and uses the deposit currency', async ({ page }) => {
  await page.route('**/api/db/transactions', async route => {
    if (route.request().method() !== 'POST') return route.fallback();
    await route.fulfill({ json: { duplicate: false } });
  });
  await page.goto('/transactions');
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  const form = page.locator('.transaction-editor');
  await form.getByLabel('Type', { exact: true }).selectOption('transfer');
  await form.getByLabel('To account', { exact: true }).click();
  await page.getByRole('option', { name: 'USD travel wallet · USD', exact: true }).filter({ visible: true }).last().click();
  await form.getByLabel('Type', { exact: true }).selectOption('income');
  await expect(form.getByLabel('Currency', { exact: true })).toHaveValue('USD');
  await form.getByLabel('Description', { exact: true }).fill('Extra income received');
  await form.getByLabel('Income treatment', { exact: true }).selectOption('refund');
  await form.getByLabel('Amount', { exact: true }).fill('15');
  await form.getByRole('button', { name: 'Save and add another', exact: true }).click();
  await expect(form.getByLabel('Amount', { exact: true })).toHaveValue('');
  await expect(form.getByRole('status').filter({ hasText: 'Draft saved on this device' })).toBeVisible();
  await page.reload();
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  await expect(form.getByLabel('Type', { exact: true })).toHaveValue('income');
  await expect(form.getByLabel('Deposit account', { exact: true }).locator('xpath=ancestor::label')).toContainText('USD travel wallet');
  await expect(form.getByLabel('Currency', { exact: true })).toHaveValue('USD');
});

test('editing expands under its row and description suggestions preserve amount and date', async ({ page }) => {
  await page.goto('/transactions');
  const row = page.getByRole('row').filter({ hasText: 'Weekly groceries · neighbourhood market' }).first();
  await row.getByRole('button', { name: 'Edit', exact: true }).click();
  const expanded = page.locator('.ant-table-expanded-row');
  await expect(expanded.getByRole('button', { name: 'Save changes' })).toBeVisible();
  expect(await expanded.evaluate(el => el.previousElementSibling?.textContent)).toContain('Weekly groceries');
  await expanded.getByRole('button', { name: 'Close draft' }).click();
  await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
  const form = page.locator('form.transaction-editor');
  const date = await form.locator('input[type=date]').inputValue();
  await form.getByLabel('Amount', { exact: true }).fill('19.75');
  await form.getByLabel('Description', { exact: true }).fill('Weekly groc');
  await page.getByRole('option').filter({ visible: true }).filter({ hasText: 'Weekly groceries · neighbourhood market' }).click();
  await expect(form.getByLabel('Description', { exact: true })).toHaveValue('Weekly groceries · neighbourhood market');
  await expect(form.getByLabel('Amount', { exact: true })).toHaveValue('19.75');
  await expect(form.locator('input[type=date]')).toHaveValue(date);
  await form.getByLabel('Budget category', { exact: true }).click();
  await form.getByLabel('Budget category', { exact: true }).fill('Travel');
  await expect(page.getByRole('option', { name: 'Travel fund', exact: true })).toBeVisible();
});
