import { test as base, expect } from '@playwright/test';
import { finance, frozenTime, reconciliation, transactions } from './finance';

export const test = base.extend<{ authenticated: void }>({
  authenticated: [async ({ page }, use) => {
    // Freeze Date while allowing timers, React effects and chart layout to run.
    await page.clock.setFixedTime(new Date(frozenTime));
    const unexpected: string[] = [];
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/api/**', async route => {
      const request = route.request();
      const url = new URL(request.url());
      let body: unknown;
      if (request.method() === 'GET' && url.pathname === '/api/session') {
        body = { authenticated: true, email: null, pocketAuthenticated: true, pocketUsername: 'Visual household' };
      } else if (request.method() === 'GET' && url.pathname === '/api/db/finance' && url.searchParams.get('asOf') === finance.asOf) {
        body = finance;
      } else if (request.method() === 'GET' && url.pathname === '/api/db/transactions') {
        const search = (url.searchParams.get('search') ?? '').toLowerCase();
        const rows = transactions.filter(row => (!search || row.description.toLowerCase().includes(search))
          && (!url.searchParams.get('kind') || row.kind === url.searchParams.get('kind'))
          && (!url.searchParams.get('category') || row.fromCategory === url.searchParams.get('category'))
          && (!url.searchParams.get('account') || row.fromAccount === url.searchParams.get('account') || row.toAccount === url.searchParams.get('account')));
        const pageNumber = Number(url.searchParams.get('page') ?? 1);
        const pageSize = Number(url.searchParams.get('pageSize') ?? 25);
        body = { transactions: rows.slice((pageNumber - 1) * pageSize, pageNumber * pageSize), total: rows.length, page: pageNumber, pageSize };
      } else if (request.method() === 'GET' && url.pathname === '/api/db/reconciliation/preview') {
        body = reconciliation;
      } else {
        unexpected.push(`${request.method()} ${url.pathname}${url.search}`);
        await route.fulfill({ status: 501, json: { error: 'Unconfigured visual fixture endpoint' } });
        return;
      }
      await route.fulfill({ json: body });
    });
    await use();
    expect(unexpected, 'Every API request must use an explicit fixture').toEqual([]);
    expect(errors, 'Authenticated pages must render without JavaScript errors').toEqual([]);
  }, { auto: true }],
});
export { expect };
