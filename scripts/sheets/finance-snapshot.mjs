import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import {
  finish,
  option,
  parseArgs,
  runtime,
} from './lib.mjs';
import { TRANSACTION_HEADERS } from './finance-schema.mjs';

function populated(value) {
  return value !== undefined && value !== null && value !== '';
}

function serialToIso(serial) {
  if (!Number.isFinite(Number(serial))) return null;
  return new Date(Date.UTC(1899, 11, 30) + Number(serial) * 86_400_000).toISOString().slice(0, 10);
}

function monthKey(isoDate) {
  return String(isoDate).slice(0, 7);
}

function transactionObject(row, formattedRow, rowNumber) {
  return {
    row: rowNumber,
    date: serialToIso(row[0]) ?? formattedRow[0] ?? null,
    description: row[1] ?? '',
    amount: row[2] ?? null,
    currency: row[3] ?? null,
    toAmount: row[4] ?? null,
    fromCategory: row[5] ?? null,
    toCategory: row[6] ?? null,
    fromAccount: row[7] ?? null,
    toAccount: row[8] ?? null,
    fromAccountCurrency: row[9] ?? null,
    fromExchangeEur: row[10] ?? null,
    fromAmountEur: row[11] ?? null,
    fromNativeExchange: row[12] ?? null,
    fromAmountNative: row[13] ?? null,
    toAccountCurrency: row[14] ?? null,
    toExchangeEur: row[15] ?? null,
    toAmountEur: row[16] ?? null,
  };
}

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const { sheets, spreadsheetId } = await runtime(args);
  const asOf = String(option(args, 'as-of', new Date().toISOString().slice(0, 10)));
  const recentCount = Number(option(args, 'recent', 50));
  if (!/^\d{4}-\d{2}-\d{2}$/.test(asOf)) throw new Error('--as-of must use YYYY-MM-DD.');
  if (!Number.isInteger(recentCount) || recentCount < 0) throw new Error('--recent must be a non-negative integer.');

  const ranges = ['Transactions!A:Q', "'Expenses v3'!A:AP", 'Accounts!A:R'];
  const [rawResponse, formattedResponse] = await Promise.all([
    sheets.spreadsheets.values.batchGet({
      spreadsheetId, ranges, majorDimension: 'ROWS', valueRenderOption: 'UNFORMATTED_VALUE',
    }),
    sheets.spreadsheets.values.batchGet({
      spreadsheetId, ranges, majorDimension: 'ROWS', valueRenderOption: 'FORMATTED_VALUE',
    }),
  ]);
  const raw = rawResponse.data.valueRanges?.map((range) => range.values ?? []) ?? [];
  const formatted = formattedResponse.data.valueRanges?.map((range) => range.values ?? []) ?? [];
  const [transactionRows, budgetRows, accountRows] = raw;
  const [formattedTransactionRows] = formatted;
  if (JSON.stringify((transactionRows ?? [])[0] ?? []) !== JSON.stringify(TRANSACTION_HEADERS)) {
    throw new Error('Transactions header fingerprint has changed; refusing to build the finance snapshot.');
  }

  const records = (transactionRows ?? [])
    .slice(1)
    .map((row, index) => ({ row, formatted: formattedTransactionRows?.[index + 1] ?? [], rowNumber: index + 2 }))
    .filter(({ row }) => row.slice(0, 9).some(populated));
  const recentTransactions = records
    .slice(Math.max(0, records.length - recentCount))
    .reverse()
    .map(({ row, formatted: formattedRow, rowNumber }) => transactionObject(row, formattedRow, rowNumber));

  const budgetHeaders = budgetRows?.[0] ?? [];
  const budgetDates = budgetRows?.[1] ?? [];
  const targetMonth = monthKey(asOf);
  const expenseColumn = budgetHeaders.findIndex((header, index) => (
    header === 'Expense' && monthKey(serialToIso(budgetDates[index]) ?? '') === targetMonth
  ));
  if (expenseColumn === -1) throw new Error(`No Expenses v3 month column found for ${targetMonth}.`);
  const budgets = (budgetRows ?? []).slice(2)
    .filter((row) => populated(row[0]))
    .map((row, index) => ({
      row: index + 3,
      name: String(row[0]),
      baseBudgetEur: row[1] ?? null,
      baseBudgetPhp: row[2] ?? null,
      expenseEur: row[expenseColumn] ?? null,
      remainingEur: row[expenseColumn + 1] ?? null,
      isReal: Boolean(row[41]),
    }));

  const accounts = (accountRows ?? []).slice(1)
    .filter((row) => populated(row[0]))
    .map((row, index) => ({
      row: index + 2,
      name: String(row[0]),
      type: row[1] ?? null,
      isReal: Boolean(row[2]),
      currency: row[3] ?? null,
      exchangeRateEur: row[4] ?? null,
      credits: row[6] ?? null,
      debits: row[7] ?? null,
      balanceNative: row[8] ?? null,
      actualBalanceNative: row[9] ?? null,
      reconciliationNative: row[10] ?? null,
      balanceEur: row[11] ?? null,
      actualBalanceEur: row[12] ?? null,
      isActive: row[17] === undefined ? Boolean(row[2]) : Boolean(row[17]),
    }));

  const report = {
    generatedAt: new Date().toISOString(),
    spreadsheetId,
    asOf,
    budgetMonth: targetMonth,
    transactionCount: records.length,
    recentTransactions,
    budgets,
    accounts,
  };
  const output = path.resolve(
    process.cwd(),
    String(option(args, 'output', '.google/finance-snapshot.json')),
  );
  await mkdir(path.dirname(output), { recursive: true });
  await writeFile(output, JSON.stringify(report, null, 2), 'utf8');
  console.log(`Finance snapshot saved to ${output}`);
  console.log(
    `${records.length} transaction(s), ${budgets.length} budget category row(s), `
      + `${budgets.filter((budget) => budget.isReal).length} selectable budget(s), `
      + `${accounts.length} account row(s), `
      + `${accounts.filter((account) => account.isReal && account.isActive).length} active real account(s); `
      + `active budget month ${targetMonth}.`,
  );
  console.log('The snapshot contains personal financial data and is intentionally stored under .google/.');
});
