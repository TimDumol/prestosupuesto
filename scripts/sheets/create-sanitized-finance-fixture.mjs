import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { TRANSACTION_FULL_HEADERS } from './finance-schema.mjs';

const DEFAULT_OUTPUT = 'fixtures/canonical-db/finance-export';
const ACCOUNT_NAMES = [
  ['Fixture EUR account', 'EUR', true],
  ['Fixture USD account', 'USD', true],
  ['Expense', 'EUR', false],
  ['Income', 'EUR', false],
  ['Reallocation', 'EUR', false],
  ['Reconciliation', 'EUR', false],
];
const CATEGORY_ROWS = [
  ['Fixture Groceries', true],
  ['Fixture Savings', true],
  ['Expense', false],
  ['Income', false],
  ['Reallocation', false],
  ['Reconciliation', false],
  ['Fixture Transfers', false],
];

function usage() {
  console.log('Usage: node scripts/sheets/create-sanitized-finance-fixture.mjs <private-test-export-dir> [output-dir]');
}

function readCell(cell, key = 'effective') {
  const value = cell?.[key];
  if (!value) return null;
  for (const kind of ['stringValue', 'numberValue', 'boolValue']) {
    if (Object.hasOwn(value, kind)) return value[kind];
  }
  return null;
}

function makeCell(entered, effective = entered, formatted = null) {
  return { entered, effective, formatted, dataSourceFormula: null };
}

function textCell(value) {
  return makeCell({ stringValue: String(value) });
}

function numberCell(value) {
  return makeCell({ numberValue: Number(value) });
}

function formulaCell(formula, value) {
  return makeCell({ formulaValue: formula }, typeof value === 'string'
    ? { stringValue: value }
    : { numberValue: Number(value) });
}

function blankRow(width) {
  return Array.from({ length: width }, () => null);
}

function serial(date) {
  return (Date.parse(`${date}T00:00:00.000Z`) - Date.UTC(1899, 11, 30)) / 86_400_000;
}

function headerLabel(cell) {
  return String(readCell(cell, 'effective') ?? readCell(cell, 'entered') ?? '');
}

function headerIndex(headers, name) {
  const result = headers.findIndex((cell) => headerLabel(cell) === name);
  if (result < 0) throw new Error('Source test export does not match the required header contract.');
  return result;
}

function translateFormula(formula, sourceRow, targetRow) {
  const rowReference = new RegExp(`(?<![A-Za-z0-9_!])(\\$?[A-Z]{1,3}\\$?)${sourceRow}(?![0-9])`, 'g');
  return formula.replace(rowReference, `$1${targetRow}`);
}

function sourceFormula(sheet, column) {
  for (let index = 1; index < sheet.rows.length; index += 1) {
    const formula = sheet.rows[index]?.[column]?.entered?.formulaValue;
    if (typeof formula === 'string' && formula.startsWith('=')) {
      return { formula, rowNumber: index + 1 };
    }
  }
  throw new Error('The test export does not contain a formula template for the sanitized fixture.');
}

function financeAmount(date, amount, currency, overrides = {}) {
  const rowNumber = overrides.rowNumber;
  const row = blankRow(20);
  row[0] = numberCell(serial(date));
  row[1] = textCell(overrides.description ?? 'synthetic fixture entry');
  row[2] = numberCell(amount);
  if (overrides.currencyOverride) row[3] = textCell(currency);
  else row[3] = formulaCell(overrides.currencyFormula, currency);
  if (overrides.toAmountOverride) row[4] = numberCell(overrides.toAmount);
  else row[4] = formulaCell(overrides.toAmountFormula, overrides.toAmount ?? amount);
  row[5] = textCell(overrides.fromCategory);
  row[6] = textCell(overrides.toCategory);
  row[7] = textCell(overrides.fromAccount);
  row[8] = textCell(overrides.toAccount);

  const accountCurrency = overrides.accountCurrency ?? currency;
  const destinationCurrency = overrides.destinationCurrency ?? accountCurrency;
  const destinationAmount = overrides.toAmount ?? amount;
  const eurRate = accountCurrency === 'EUR' ? 1 : accountCurrency === 'USD' ? 1.1 : 0.02;
  const destinationRate = destinationCurrency === 'EUR' ? 1 : destinationCurrency === 'USD' ? 1.1 : 0.02;
  const outputs = [
    accountCurrency,
    eurRate,
    amount * eurRate,
    1,
    amount,
    destinationCurrency,
    destinationRate,
    destinationAmount * destinationRate,
  ];
  for (let outputIndex = 0; outputIndex < 8; outputIndex += 1) {
    const sourceColumn = outputIndex + 9;
    const formula = overrides.outputOverrides?.[outputIndex]
      ?? translateFormula(overrides.outputFormulas[outputIndex].formula,
        overrides.outputFormulas[outputIndex].rowNumber, rowNumber);
    const value = outputs[outputIndex];
    row[sourceColumn] = formulaCell(formula, value);
  }
  if (overrides.historicalGap) row[9] = null;
  if (overrides.historicalError) {
    row[16] = makeCell(
      { formulaValue: translateFormula(overrides.outputFormulas[7].formula,
        overrides.outputFormulas[7].rowNumber, rowNumber) },
      { errorValue: { type: 'ERROR', message: '#N/A' } },
    );
  }
  if (overrides.transactionId) row[17] = textCell(overrides.transactionId);
  return row;
}

async function main() {
  const [inputArg, outputArg] = process.argv.slice(2);
  if (!inputArg || inputArg === '--help' || inputArg === '-h') {
    usage();
    if (!inputArg || inputArg === '--help' || inputArg === '-h') process.exitCode = inputArg ? 0 : 1;
    return;
  }
  const input = path.resolve(process.cwd(), inputArg);
  const output = path.resolve(process.cwd(), outputArg ?? DEFAULT_OUTPUT);
  const manifest = JSON.parse(await readFile(path.join(input, 'manifest.json'), 'utf8'));
  const archiveBytes = await readFile(path.join(input, manifest.exportFile));
  if (createHash('sha256').update(archiveBytes).digest('hex') !== manifest.exportSha256) {
    throw new Error('Test export digest does not match its manifest.');
  }
  const archive = JSON.parse(archiveBytes.toString('utf8'));
  if (manifest.testWorkbook !== true || archive.source.spreadsheetId !== manifest.sourceSpreadsheetId) {
    throw new Error('Sanitized fixture generation requires an explicitly exported test workbook.');
  }
  const txSource = archive.sheets.transactions;
  const accountSource = archive.sheets.accounts;
  const budgetSource = archive.sheets.budgets;
  const rateSource = archive.sheets.exchangeRates;
  const txHeaders = txSource.rows[0];
  if (JSON.stringify(txHeaders.map(headerLabel)) !== JSON.stringify(TRANSACTION_FULL_HEADERS)) {
    throw new Error('Test workbook transaction headers changed; fixture generation refused.');
  }
  const accountHeaders = accountSource.rows[0].slice(0, 18);
  for (const name of ['Name', 'Type', 'IsReal', 'Currency', 'Balance', 'Actual Balance',
    'Reconciliation Amount', 'Balance (EUR)', 'Actual Balance (EUR)']) {
    headerIndex(accountHeaders, name);
  }
  const activeColumn = accountHeaders.findIndex((cell) => ['IsActive', 'IsLive'].includes(headerLabel(cell)));
  if (activeColumn < 0) throw new Error('Test workbook has no active-account header.');

  const formulas = [3, 4, 9, 10, 11, 12, 13, 14, 15, 16]
    .map((column) => sourceFormula(txSource, column));
  const formulaByColumn = new Map([3, 4, 9, 10, 11, 12, 13, 14, 15, 16]
    .map((column, index) => [column, formulas[index]]));
  const formulaFor = (column, rowNumber) => translateFormula(
    formulaByColumn.get(column).formula,
    formulaByColumn.get(column).rowNumber,
    rowNumber,
  );
  const outputFormulas = Array.from({ length: 8 }, (_, index) => formulaByColumn.get(index + 9));

  const transactions = [txHeaders];
  const descriptions = [
    'CANONICAL-DB-PHASE-B-FIXTURE-20260928-EXPENSE',
    'CANONICAL-DB-PHASE-B-FIXTURE-20260928-INCOME',
    'CANONICAL-DB-PHASE-B-FIXTURE-20260928-CROSS-CURRENCY',
    'CANONICAL-DB-PHASE-B-FIXTURE-20260928-REALLOCATION',
    'CANONICAL-DB-PHASE-B-FIXTURE-20260928-RECONCILIATION',
    'CANONICAL-DB-PHASE-B-FIXTURE-20260928-ZERO-RECONCILIATION',
    'CANONICAL-DB-PHASE-B-FIXTURE-20260928-HISTORICAL-FORMULA-EXCEPTION',
  ];
  const rows = [
    { date: '2026-09-01', amount: 12.34, currency: 'EUR', kind: 'expense' },
    { date: '2026-09-02', amount: 25, currency: 'EUR', kind: 'income' },
    { date: '2026-09-03', amount: 5, currency: 'USD', toAmount: 4.5, kind: 'transfer' },
    { date: '2026-09-04', amount: 2.75, currency: 'EUR', kind: 'reallocate' },
    { date: '2026-09-05', amount: -1.25, currency: 'EUR', kind: 'reconciliation' },
    { date: '2026-09-06', amount: 0, currency: 'EUR', kind: 'reconciliation' },
    { date: '2025-05-15', amount: 1.11, currency: 'EUR', kind: 'historical' },
  ];
  for (let index = 0; index < rows.length; index += 1) {
    const spec = rows[index];
    const rowNumber = index + 2;
    const base = {
      rowNumber,
      description: descriptions[index],
      currencyFormula: formulaFor(3, rowNumber),
      toAmountFormula: formulaFor(4, rowNumber),
      outputFormulas,
      accountCurrency: spec.kind === 'transfer' ? 'USD' : 'EUR',
      destinationCurrency: 'EUR',
      toAmount: spec.toAmount ?? spec.amount,
      transactionId: index === 0 ? '0198ca00-0000-7000-8000-000000000001' : null,
    };
    if (spec.kind === 'expense') Object.assign(base, {
      fromCategory: 'Fixture Groceries', toCategory: 'Expense',
      fromAccount: 'Fixture EUR account', toAccount: 'Expense',
    });
    if (spec.kind === 'income') Object.assign(base, {
      fromCategory: 'Income', toCategory: 'Fixture Groceries',
      fromAccount: 'Income', toAccount: 'Fixture EUR account',
    });
    if (spec.kind === 'transfer') Object.assign(base, {
      fromCategory: 'Fixture Transfers', toCategory: 'Fixture Transfers',
      fromAccount: 'Fixture USD account', toAccount: 'Fixture EUR account',
      currencyOverride: true, toAmountOverride: true,
    });
    if (spec.kind === 'reallocate') Object.assign(base, {
      fromCategory: 'Fixture Groceries', toCategory: 'Fixture Savings',
      fromAccount: 'Reallocation', toAccount: 'Reallocation',
    });
    if (spec.kind === 'reconciliation') Object.assign(base, {
      fromCategory: 'Reconciliation', toCategory: 'Reconciliation',
      fromAccount: 'Fixture EUR account', toAccount: 'Reconciliation',
    });
    if (spec.kind === 'historical') Object.assign(base, {
      fromCategory: 'Fixture Groceries', toCategory: 'Expense',
      fromAccount: 'Fixture EUR account', toAccount: 'Expense',
      currencyOverride: true, historicalGap: true, historicalError: true,
    });
    const row = financeAmount(spec.date, spec.amount, spec.currency, base);
    transactions.push(row);
  }

  const accountRows = [accountHeaders];
  const indexes = Object.fromEntries(['Name', 'Type', 'IsReal', 'Currency', 'Balance', 'Actual Balance',
    'Reconciliation Amount', 'Balance (EUR)', 'Actual Balance (EUR)'].map((name) => [name, headerIndex(accountHeaders, name)]));
  for (const [name, currency, isReal] of ACCOUNT_NAMES) {
    const row = blankRow(18);
    row[indexes.Name] = textCell(name);
    row[indexes.Type] = textCell(name.includes('account') ? 'checking' : 'system');
    row[indexes.IsReal] = makeCell({ boolValue: isReal });
    row[indexes.Currency] = textCell(currency);
    row[indexes.Balance] = numberCell(100);
    row[indexes['Actual Balance']] = numberCell(100);
    row[indexes['Reconciliation Amount']] = numberCell(0);
    row[indexes['Balance (EUR)']] = numberCell(currency === 'EUR' ? 100 : 110);
    row[indexes['Actual Balance (EUR)']] = numberCell(currency === 'EUR' ? 100 : 110);
    row[activeColumn] = makeCell({ boolValue: isReal });
    accountRows.push(row);
  }

  const checkpointMonths = [];
  for (let year = 2023; year <= 2026; year += 1) {
    const firstMonth = year === 2023 ? 9 : 1;
    const lastMonth = year === 2026 ? 9 : 12;
    for (let month = firstMonth; month <= lastMonth; month += 1) {
      const monthKey = `${year}-${String(month).padStart(2, '0')}`;
      if (!['2024-11', '2026-05'].includes(monthKey)) checkpointMonths.push(monthKey);
    }
  }
  const checkpointRows = [checkpointMonths.map((month) => numberCell(serial(`${month}-01`)))];
  for (let accountIndex = 1; accountIndex < accountRows.length; accountIndex += 1) {
    checkpointRows.push(blankRow(35));
  }
  checkpointRows[1][0] = numberCell(88.23);
  checkpointRows[1][34] = formulaCell('=100', 100);
  checkpointRows[2][4] = makeCell(
    { formulaValue: '=100' },
    { errorValue: { type: 'ERROR', message: '#N/A' } },
  );
  checkpointRows[2][34] = numberCell(123.45);

  const budgetHeaders = [
    textCell('Category'), textCell('Monthly Budget EUR'), textCell('Monthly Budget PHP'),
    textCell('Expense'), textCell('Remaining'), textCell('Expense'), textCell('Remaining'), textCell('IsReal'),
  ];
  const budgetDates = [null, null, null, numberCell(serial('2025-05-01')), numberCell(serial('2025-05-01')),
    numberCell(serial('2026-09-01')), numberCell(serial('2026-09-01')), null];
  const budgetRows = [budgetHeaders, budgetDates];
  for (const [name, isReal] of CATEGORY_ROWS) {
    const row = blankRow(8);
    row[0] = textCell(name);
    row[1] = numberCell(isReal ? 500 : 0);
    row[2] = numberCell(isReal ? 25_000 : 0);
    row[3] = numberCell(isReal ? 6 : 0);
    row[4] = numberCell(isReal ? 494 : 0);
    row[5] = formulaCell('=0', isReal ? 12.34 : 0);
    row[6] = formulaCell('=0', isReal ? 481.66 : 0);
    row[7] = makeCell({ boolValue: isReal });
    budgetRows.push(row);
  }

  const rateRows = [
    [textCell('PHPEUR'), null, textCell('USDEUR'), null, textCell('PHPUSD'), null],
    [textCell('Date'), textCell('Close'), textCell('Date'), textCell('Close'), textCell('Date'), textCell('Close')],
    [numberCell(serial('2026-08-31')), numberCell(0.02), numberCell(serial('2026-08-31')), numberCell(0.91), numberCell(serial('2026-08-31')), numberCell(55)],
    [numberCell(serial('2026-09-07')), numberCell(0.021), numberCell(serial('2026-09-07')), numberCell(0.92), numberCell(serial('2026-09-07')), numberCell(54)],
  ];
  const synthetic = {
    schemaVersion: 1,
    source: { spreadsheetId: 'SANITIZED-PHASE-B-FIXTURE' },
    exportedAt: '2026-09-28T00:00:00.000Z',
    canonicalTimeZone: 'Europe/Madrid',
    workbook: { locale: 'en_US', timeZone: 'Asia/Manila' },
    sheets: {
      transactions: sheet('Transactions', 1, 20, transactions),
      accounts: sheet('Accounts', 2, 18, accountRows),
      accountCheckpoints: accountCheckpointSheet(checkpointRows),
      budgets: sheet('Expenses v3', 3, 8, budgetRows),
      exchangeRates: sheet('Exchange Rates', 4, 6, rateRows),
    },
  };
  const bytes = `${JSON.stringify(synthetic)}\n`;
  const exportSha256 = createHash('sha256').update(bytes).digest('hex');
  const formulaStates = summarizeFormulaStates(transactions, 9, 17);
  const fixtureManifest = {
    schemaVersion: 1,
    sourceSpreadsheetId: synthetic.source.spreadsheetId,
    exportFile: 'finance-export.json',
    exportSha256,
    exportedAt: synthetic.exportedAt,
    canonicalTimeZone: 'Europe/Madrid',
    currentCanonicalMonth: '2026-09',
    workbook: synthetic.workbook,
    sheets: Object.fromEntries(Object.entries(synthetic.sheets).map(([key, value]) => [key, {
      title: value.title,
      sheetId: value.sheetId,
      range: value.range,
      rowCount: value.rowCount,
      columnCount: value.columnCount,
    }])),
    transactionValuationCellStates: formulaStates,
    accountCheckpointCellStates: summarizeFormulaStates(checkpointRows),
    testWorkbook: true,
    sanitizedFixture: true,
  };
  await mkdir(output, { recursive: true });
  await writeFile(path.join(output, 'finance-export.json'), bytes, { encoding: 'utf8', mode: 0o644, flag: 'wx' });
  await writeFile(path.join(output, 'manifest.json'), `${JSON.stringify(fixtureManifest, null, 2)}\n`, {
    encoding: 'utf8', mode: 0o644, flag: 'wx',
  });
  console.log(`Sanitized finance fixture created at ${path.relative(process.cwd(), output)}.`);
  console.log(`Synthetic transactions: ${transactions.length - 1}; budget categories: ${CATEGORY_ROWS.length}; historical account columns: ${checkpointMonths.length}; synthetic rate observations: 6.`);
}

function accountCheckpointSheet(rows) {
  const allRows = rows.map((row) => Array.from({ length: 35 }, (_, index) => row[index] ?? null));
  let lastUsed = allRows.length - 1;
  while (lastUsed > 0 && allRows[lastUsed].every((cell) => cell === null)) lastUsed -= 1;
  const paddedRows = allRows.slice(0, lastUsed + 1);
  return {
    title: 'Accounts',
    sheetId: 2,
    range: `'Accounts'!W1:BE${allRows.length}`,
    gridRowCount: allRows.length,
    gridColumnCount: 57,
    columnCount: 35,
    rowCount: paddedRows.length,
    rows: paddedRows,
  };
}

function sheet(title, sheetId, columnCount, rows) {
  const paddedRows = rows.map((row) => Array.from({ length: columnCount }, (_, index) => row[index] ?? null));
  return {
    title,
    sheetId,
    range: `'${title}'!A1:${columnName(columnCount)}${paddedRows.length}`,
    gridRowCount: paddedRows.length,
    gridColumnCount: columnCount,
    columnCount,
    rowCount: paddedRows.length,
    rows: paddedRows,
  };
}

function columnName(mutValue) {
  let value = mutValue;
  let result = '';
  while (value > 0) {
    value -= 1;
    result = String.fromCharCode(65 + (value % 26)) + result;
    value = Math.floor(value / 26);
  }
  return result;
}

function summarizeFormulaStates(rows, firstColumn = 0, endColumn = 35) {
  const summary = { formula: 0, literal: 0, blank: 0, error: 0 };
  for (const row of rows.slice(1)) {
    for (const cell of row.slice(firstColumn, endColumn)) {
      if (!cell) summary.blank += 1;
      else if (cell.effective?.errorValue) summary.error += 1;
      else if (cell.entered?.formulaValue !== undefined) summary.formula += 1;
      else if (cell.entered) summary.literal += 1;
      else summary.blank += 1;
    }
  }
  return { summary, errors: [] };
}

main().catch((error) => {
  console.error(`Fixture generation failed: ${error?.message ?? String(error)}`);
  process.exitCode = 1;
});
