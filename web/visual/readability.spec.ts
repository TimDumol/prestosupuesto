import { test, expect } from './test';
import type { Page } from '@playwright/test';

async function expectReadableText(page: Page) {
  const undersized = await page.evaluate(() => {
    const failures: string[] = [];
    const inspect = (element: Element, text: string) => {
      if (!text.trim() || !element.getClientRects().length) return;
      for (let ancestor: Element | null = element; ancestor; ancestor = ancestor.parentElement) {
        const style = getComputedStyle(ancestor);
        if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') return;
        if (ancestor instanceof HTMLDetailsElement && !ancestor.open && !ancestor.querySelector('summary')?.contains(element)) return;
      }
      const style = getComputedStyle(element);
      const font = parseFloat(style.fontSize);
      if (font < 16) failures.push(`${element.tagName}.${element.getAttribute('class') ?? ''}: ${font}px — ${text.trim().slice(0, 100)}`);
    };
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      if (node.parentElement && !['SCRIPT', 'STYLE', 'OPTION', 'TITLE'].includes(node.parentElement.tagName)) inspect(node.parentElement, node.textContent ?? '');
    }
    document.querySelectorAll('input, textarea, select').forEach(element => inspect(element, 'form control'));
    return [...new Set(failures)];
  });
  expect(undersized, 'Visible text and controls must be at least 16px, including SVG charts and library portals').toEqual([]);
}

for (const path of ['/', '/budgets', '/history', '/reconciliation', '/transactions']) {
  test(`readable text throughout ${path}`, async ({ page }) => {
    await page.goto(path);
    await expect(page.locator('.page-intro h1')).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    await expectReadableText(page);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), 'Page content should fit without horizontal overflow').toBe(true);
    if (path === '/') {
      await page.locator('.funding-details summary').click();
      await expectReadableText(page);
      await page.getByRole('textbox', { name: 'Reporting month' }).click();
      await expect(page.locator('.react-datepicker')).toBeVisible();
      await expectReadableText(page);
    }
    if (path === '/budgets') {
      await page.locator('.budget-category-select').first().click();
      await expect(page.locator('.ant-select-dropdown')).toBeVisible();
      await expectReadableText(page);
    }
    if (path === '/history') {
      await page.getByRole('slider').first().focus();
      await expect(page.locator('.visx-tooltip')).toBeVisible();
      await expectReadableText(page);
    }
    if (path === '/transactions') {
      await page.getByRole('button', { name: '＋ Add / resume transaction' }).click();
      await page.locator('.transaction-details summary').click();
      await expectReadableText(page);
    }
  });
}

test('budget mix labels and notes follow a doubled browser font preference', async ({ page }) => {
  await page.goto('/budgets');
  const mixes = page.locator('.budget-composition-grid');
  await expect(mixes).toBeVisible();
  await page.evaluate(() => document.documentElement.style.fontSize = '200%');
  const fonts = await mixes.locator('.budget-composition-label, .budget-composition-label > small, .budget-composition-note').evaluateAll(elements => elements.map(element => parseFloat(getComputedStyle(element).fontSize)));
  expect(fonts.length).toBeGreaterThan(0);
  expect(fonts.every(size => size >= 32)).toBe(true);
});

test('signed-out text remains readable', async ({ page }) => {
  await page.route('**/api/session', route => route.fulfill({ json: { authenticated: false, pocketAuthenticated: false } }));
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Continue with Pocket ID' })).toBeVisible();
  await expectReadableText(page);
});
