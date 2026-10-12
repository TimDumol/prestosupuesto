import { test, expect } from './test';
import { fileURLToPath } from 'node:url';

test('monthly plan edits show totals, validate amounts and can be discarded', async ({ page }) => {
  await page.goto('/budgets');
  await page.getByRole('button', { name: 'Edit monthly budget' }).click();
  const allocation = page.getByRole('textbox', { name: 'Groceries monthly allocation' });
  await expect(allocation).toBeVisible();
  await allocation.fill('-1');
  await expect(page.getByRole('button', { name: 'Save monthly budget' })).toBeDisabled();
  await allocation.fill('123,45');
  await expect(page.getByRole('button', { name: 'Save monthly budget' })).toBeEnabled();
  await allocation.blur();
  await page.evaluate(() => document.fonts.ready);
  await expect(page).toHaveScreenshot('monthly-budget-editor.png', { fullPage: true });
  await page.getByRole('button', { name: 'Discard changes' }).click();
  await expect(allocation).toHaveCount(0);
});

test('create a category with a monthly allocation', async ({ page }) => {
  let posted: unknown;
  await page.route('**/api/db/finance/categories', async route => {
    posted = route.request().postDataJSON();
    await route.fulfill({ json: { categoryCreated: true } });
  });
  await page.goto('/budgets');
  await page.getByRole('button', { name: '+ New category' }).click();
  await page.getByRole('textbox', { name: 'Category name' }).fill('Groceries');
  await expect(page.getByRole('button', { name: 'Create category', exact: true })).toBeDisabled();
  await page.getByRole('textbox', { name: 'Category name' }).fill('Home maintenance');
  await page.getByRole('textbox', { name: 'Monthly allocation · EUR' }).fill('25,50');
  await expect(page).toHaveScreenshot('new-budget-category.png', { fullPage: true });
  await page.getByRole('button', { name: 'Create category', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Category created.' })).toBeVisible();
  expect(posted).toEqual({ name: 'Home maintenance', monthlyAmount: '25.5' });
});

test('new yearly fee category calculates monthly saving and aligns the fields', async ({ page }, testInfo) => {
  let posted: unknown;
  await page.route('**/api/db/finance/categories', async route => {
    posted = route.request().postDataJSON();
    await route.fulfill({ json: { categoryCreated: true } });
  });
  await page.goto('/budgets');
  await page.getByRole('button', { name: '+ New category' }).click();
  const name = page.getByRole('textbox', { name: 'Category name' });
  const monthly = page.getByRole('textbox', { name: 'Monthly allocation · EUR' });
  await name.fill('Annual membership');
  if (testInfo.project.name === 'desktop') {
    const a = await name.boundingBox();
    const b = await monthly.boundingBox();
    expect(Math.abs(a!.y - b!.y)).toBeLessThan(2);
  }
  await page.getByRole('button', { name: 'Save for a yearly fee' }).click();
  await expect(page.getByRole('button', { name: 'Create category', exact: true })).toBeDisabled();
  await page.getByRole('textbox', { name: 'Yearly fee · EUR' }).fill('120');
  await expect(monthly).toHaveValue('10');
  await page.getByLabel('Next payment month (optional)').fill('2027-01');
  await expect(monthly).toHaveValue('30');
  await page.getByRole('textbox', { name: 'Yearly fee · EUR' }).blur();
  await expect(page.locator('.new-category-form')).toHaveScreenshot('new-yearly-fee-category.png');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.getByRole('button', { name: 'Create category', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Category created.' })).toBeVisible();
  expect(posted).toEqual({ name: 'Annual membership', monthlyAmount: '30' });
});

test('yearly fee uses existing savings without counting this month allocation twice', async ({ page }) => {
  let posted: unknown;
  await page.route('**/api/db/finance/monthly-budget', async route => {
    posted = route.request().postDataJSON();
    await route.fulfill({ json: { saved: true } });
  });
  await page.goto('/budgets');
  await page.getByRole('button', { name: 'Edit monthly budget' }).click();
  await page.locator('.allocation-card').filter({ hasText: 'Travel fund' }).getByRole('button', { name: 'Yearly fee' }).click();
  await expect(page.getByLabel('Category to save in')).toHaveValue('Travel fund');
  await page.getByRole('textbox', { name: 'Yearly fee · EUR' }).fill('1200');
  await page.getByLabel('Next payment month (optional)').fill('2027-01');
  await page.getByRole('button', { name: 'Use €137.50 / month' }).click();
  await expect(page.getByRole('textbox', { name: 'Travel fund monthly allocation' })).toHaveValue('137.5');
  await page.getByLabel('Find a category').fill('Travel');
  await expect(page.locator('.allocation-card')).toHaveCount(1);
  await expect(page.locator('.monthly-plan-editor')).toHaveScreenshot('yearly-fee-budget-editing.png', {
    // A tall element screenshot scrolls beneath the phone's sticky navigation.
    stylePath: fileURLToPath(new URL('./form-screenshot.css', import.meta.url)),
  });
  await page.getByRole('button', { name: 'Save monthly budget' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Monthly budget saved.' })).toBeVisible();
  expect(posted).toEqual({ effectiveStartMonth: '2026-10', categories: [{ name: 'Travel fund', amount: '137.5' }] });
});
