import { createHash } from 'node:crypto';
import { mkdir, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import {
  extractSpreadsheetId,
  finish,
  option,
  parseArgs,
  runtime,
} from './lib.mjs';
import {
  TRANSACTION_FULL_HEADERS,
  TRANSACTION_HEADERS,
} from './finance-schema.mjs';

const CANONICAL_TIME_ZONE = 'Europe/Madrid';
const DEFAULT_SHEETS = {
  transactions: 'Transactions',
  accounts: 'Accounts',
  budgets: 'Expenses v3',
  exchangeRates: 'Exchange Rates',
};
const ACCOUNT_HEADERS = [
  'Name', 'Type', 'IsReal', 'Currency', 'Balance', 'Actual Balance',
  'Reconciliation Amount', 'Balance (EUR)', 'Actual Balance (EUR)',
];

function cellValue(cell, field) {
  const value = cell?.[field];
  if (!value) return null;
  if (Object.hasOwn(value, 'numberValue')) return value.numberValue;
  if (Object.hasOwn(value, 'stringValue')) return value.stringValue;
  if (Object.hasOwn(value, 'boolValue')) return value.boolValue;
  if (Object.hasOwn(value, 'formulaValue')) return value.formulaValue;
  if (Object.hasOwn(value, 'errorValue')) return value.errorValue;
  return value;
}

function serializeCell(cell) {
  if (!cell) return null;
  const userEnteredValue = cell.userEnteredValue ?? null;
  const effectiveValue = cell.effectiveValue ?? null;
  if (!userEnteredValue && !effectiveValue && cell.formattedValue === undefined && !cell.dataSourceFormula) return null;
  return {
    entered: userEnteredValue,
    effective: effectiveValue,
    formatted: cell.formattedValue ?? null,
    dataSourceFormula: cell.dataSourceFormula ?? null,
  };
}

function cellText(cell) {
  const effective = cellValue(cell, 'effective');
  if (effective !== null) return String(effective);
  const entered = cellValue(cell, 'entered');
  return entered === null ? '' : String(entered);
}

function normalizeRows(gridData, columnCount) {
  const rows = (gridData?.rowData ?? []).map((row) => (
    Array.from({ length: columnCount }, (_, column) => serializeCell(row.values?.[column]))
  ));
  let lastUsed = rows.length - 1;
  while (lastUsed >= 0 && rows[lastUsed].every((cell) => cell === null)) lastUsed -= 1;
  return rows.slice(0, lastUsed + 1);
}

function firstRow(rows) {
  return rows[0] ?? [];
}

function assertTransactionHeaders(rows) {
  const headers = firstRow(rows).map((cell) => cellText(cell));
  if (JSON.stringify(headers.slice(0, TRANSACTION_HEADERS.length)) !== JSON.stringify(TRANSACTION_HEADERS)
    || JSON.stringify(headers.slice(TRANSACTION_HEADERS.length, TRANSACTION_FULL_HEADERS.length))
      !== JSON.stringify(TRANSACTION_FULL_HEADERS.slice(TRANSACTION_HEADERS.length))) {
    throw new Error('Transactions A:T headers changed; export refused.');
  }
}

function assertAccountsHeaders(rows) {
  const headers = firstRow(rows).map((cell) => cellText(cell));
  const activeHeader = headers.includes('IsActive') ? 'IsActive' : 'IsLive';
  for (const required of ACCOUNT_HEADERS) {
    if (!headers.includes(required)) throw new Error(`Accounts is missing a required header (${required}); export refused.`);
  }
  if (!headers.includes(activeHeader)) throw new Error('Accounts is missing IsActive/IsLive; export refused.');
}

function assertBudgetHeaders(rows) {
  const headers = firstRow(rows).map((cell) => cellText(cell));
  const dates = (rows[1] ?? []).map((cell) => cellValue(cell, 'effective'));
  if (!headers.includes('IsReal')) throw new Error('Expenses v3 is missing IsReal; export refused.');
  let pairs = 0;
  for (let column = 0; column < headers.length; column += 1) {
    if (headers[column] !== 'Expense') continue;
    if (headers[column + 1] !== 'Remaining' || dates[column] === null || dates[column] === undefined) {
      throw new Error(`Expenses v3 month pair at column ${column + 1} changed; export refused.`);
    }
    pairs += 1;
  }
  if (pairs === 0) throw new Error('Expenses v3 contains no dated Expense/Remaining pairs; export refused.');
}

function assertAccountCheckpointHeaders(rows) {
  const headers = firstRow(rows);
  if (headers.length !== 35 || !headers.some((cell) => cellValue(cell, 'effective') !== null)) {
    throw new Error('Accounts W:BE checkpoint headers changed; export refused.');
  }
}

function summarizeStates(rows, firstColumn, lastColumn, firstDataRow = 1) {
  const summary = { formula: 0, literal: 0, blank: 0, error: 0 };
  const errors = [];
  for (let rowIndex = firstDataRow; rowIndex < rows.length; rowIndex += 1) {
    const row = rows[rowIndex] ?? [];
    for (let column = firstColumn; column <= lastColumn; column += 1) {
      const cell = row[column] ?? null;
      const entered = cell?.entered;
      if (cell?.effective?.errorValue) {
        summary.error += 1;
        errors.push({ row: rowIndex + 1, column: column + 1 });
      } else if (entered?.formulaValue !== undefined || cell?.dataSourceFormula) summary.formula += 1;
      else if (entered && Object.keys(entered).length > 0) summary.literal += 1;
      else summary.blank += 1;
    }
  }
  return { summary, errors };
}

function localMonth(timeZone, now = new Date()) {
  const parts = new Intl.DateTimeFormat('en-CA', {
    timeZone, year: 'numeric', month: '2-digit',
  }).formatToParts(now);
  const values = Object.fromEntries(parts.map(({ type, value }) => [type, value]));
  return `${values.year}-${values.month}`;
}

function fingerprint(value) {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex');
}

function safeExportDirectory(output) {
  return stat(output).then(() => true, () => false);
}

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const testMode = Boolean(option(args, 'test', false));
  const requestedId = option(
    args,
    'spreadsheet',
    testMode ? process.env.GOOGLE_SHEETS_TEST_SPREADSHEET_ID : undefined,
  );
  if (testMode && !requestedId) throw new Error('--test requires GOOGLE_SHEETS_TEST_SPREADSHEET_ID.');
  if (!testMode && !requestedId && !process.env.GOOGLE_SHEETS_SPREADSHEET_ID) {
    throw new Error('Pass --spreadsheet or configure GOOGLE_SHEETS_SPREADSHEET_ID.');
  }
  const spreadsheetId = extractSpreadsheetId(requestedId ?? process.env.GOOGLE_SHEETS_SPREADSHEET_ID);
  const { sheets } = await runtime({ ...args, spreadsheet: spreadsheetId });
  const metadata = await sheets.spreadsheets.get({
    spreadsheetId,
    fields: 'spreadsheetId,properties(locale,timeZone),sheets(properties(sheetId,title,gridProperties(rowCount,columnCount)))',
  });
  const sheetByName = new Map((metadata.data.sheets ?? []).map((sheet) => [sheet.properties.title, sheet]));
  const requested = Object.fromEntries(Object.entries(DEFAULT_SHEETS).map(([key, fallback]) => [
    key,
    String(option(args, `${key.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)}-sheet`, fallback)),
  ]));
  const specifications = [
    ['transactions', 20], ['accounts', 18], ['budgets', 3], ['exchangeRates', 3],
  ];
  const sheetsToExport = {};
  for (const [key, minimumColumns] of specifications) {
    const title = requested[key];
    const sheet = sheetByName.get(title);
    if (!sheet) throw new Error(`Required sheet is missing (${title}); export refused.`);
    const { rowCount, columnCount } = sheet.properties.gridProperties ?? {};
    if (!Number.isInteger(rowCount) || rowCount < 1 || !Number.isInteger(columnCount) || columnCount < minimumColumns) {
      throw new Error(`Required range dimensions changed for ${title}; export refused.`);
    }
    const endColumn = key === 'transactions' ? 'T'
      : key === 'accounts' ? 'R'
        : null;
    const range = endColumn ? `'${title.replaceAll("'", "''")}'!A1:${endColumn}${rowCount}`
      : `'${title.replaceAll("'", "''")}'!A1:${columnCountName(columnCount)}${rowCount}`;
    const response = await sheets.spreadsheets.get({
      spreadsheetId,
      ranges: [range],
      includeGridData: true,
      fields: 'sheets(properties(sheetId,title,gridProperties(rowCount,columnCount)),data(startRow,startColumn,rowData(values(userEnteredValue,effectiveValue,formattedValue,dataSourceFormula))))',
    });
    const responseSheet = response.data.sheets?.[0];
    if (!responseSheet || responseSheet.properties.sheetId !== sheet.properties.sheetId) {
      throw new Error(`Could not read the required range for ${title}; export refused.`);
    }
    const width = endColumn ? (key === 'transactions' ? 20 : 18) : columnCount;
    const rows = normalizeRows(responseSheet.data?.[0], width);
    sheetsToExport[key] = {
      title,
      sheetId: sheet.properties.sheetId,
      range,
      gridRowCount: rowCount,
      gridColumnCount: columnCount,
      columnCount: width,
      rowCount: rows.length,
      rows,
    };
  }

  const accountsSheet = sheetByName.get(requested.accounts);
  const accountRowCount = accountsSheet.properties.gridProperties.rowCount;
  const checkpointsRange = `'${requested.accounts.replaceAll("'", "''")}'!W1:BE${accountRowCount}`;
  const checkpointsResponse = await sheets.spreadsheets.get({
    spreadsheetId,
    ranges: [checkpointsRange],
    includeGridData: true,
    fields: 'sheets(properties(sheetId,title,gridProperties(rowCount,columnCount)),data(startRow,startColumn,rowData(values(userEnteredValue,effectiveValue,formattedValue,dataSourceFormula))))',
  });
  const checkpointsSheet = checkpointsResponse.data.sheets?.[0];
  if (!checkpointsSheet || checkpointsSheet.properties.sheetId !== accountsSheet.properties.sheetId) {
    throw new Error(`Could not read historical account balances from ${requested.accounts}; export refused.`);
  }
  const checkpointRows = normalizeRows(checkpointsSheet.data?.[0], 35);
  assertAccountCheckpointHeaders(checkpointRows);
  while (checkpointRows.length < sheetsToExport.accounts.rowCount) {
    checkpointRows.push(Array.from({ length: 35 }, () => null));
  }
  sheetsToExport.accountCheckpoints = {
    title: requested.accounts,
    sheetId: accountsSheet.properties.sheetId,
    range: checkpointsRange,
    gridRowCount: accountRowCount,
    gridColumnCount: accountsSheet.properties.gridProperties.columnCount,
    columnCount: 35,
    rowCount: checkpointRows.length,
    rows: checkpointRows,
  };

  assertTransactionHeaders(sheetsToExport.transactions.rows);
  assertAccountsHeaders(sheetsToExport.accounts.rows);
  assertBudgetHeaders(sheetsToExport.budgets.rows);

  const now = new Date();
  const data = {
    schemaVersion: 1,
    source: { spreadsheetId },
    exportedAt: now.toISOString(),
    canonicalTimeZone: CANONICAL_TIME_ZONE,
    workbook: {
      locale: metadata.data.properties?.locale ?? null,
      timeZone: metadata.data.properties?.timeZone ?? null,
    },
    sheets: sheetsToExport,
  };
  const serialized = `${JSON.stringify(data)}\n`;
  const sha256 = createHash('sha256').update(serialized).digest('hex');
  const requestedOutput = option(args, 'output');
  const stamp = now.toISOString().replaceAll(':', '').replaceAll('-', '').replace(/\.\d{3}Z$/, 'Z');
  const defaultOutput = `.google/migration/finance-export-${stamp}-${sha256.slice(0, 8)}`;
  const output = path.resolve(process.cwd(), String(requestedOutput ?? defaultOutput));
  if (await safeExportDirectory(output)) throw new Error('Output directory already exists; exports are immutable.');
  await mkdir(output, { recursive: true, mode: 0o700 });
  const manifest = {
    schemaVersion: 1,
    sourceSpreadsheetId: spreadsheetId,
    exportFile: 'finance-export.json',
    exportSha256: sha256,
    exportedAt: data.exportedAt,
    canonicalTimeZone: CANONICAL_TIME_ZONE,
    workbook: data.workbook,
    sheets: Object.fromEntries(Object.entries(sheetsToExport).map(([key, sheet]) => [key, {
      title: sheet.title,
      sheetId: sheet.sheetId,
      range: sheet.range,
      rowCount: sheet.rowCount,
      columnCount: sheet.columnCount,
      headerSha256: fingerprint(sheet.rows[0] ?? []),
    }])),
    transactionValuationCellStates: summarizeStates(sheetsToExport.transactions.rows, 9, 16),
    accountCheckpointCellStates: summarizeStates(sheetsToExport.accountCheckpoints.rows, 0, 34),
    currentCanonicalMonth: localMonth(CANONICAL_TIME_ZONE, now),
    testWorkbook: testMode,
  };
  await writeFile(path.join(output, 'finance-export.json'), serialized, { encoding: 'utf8', mode: 0o600, flag: 'wx' });
  await writeFile(path.join(output, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, {
    encoding: 'utf8', mode: 0o600, flag: 'wx',
  });
  console.log(`Private finance export saved: ${path.relative(process.cwd(), output)}`);
  console.log(`Workbook rows: transactions ${sheetsToExport.transactions.rowCount - 1}, `
    + `accounts ${sheetsToExport.accounts.rowCount - 1}, `
    + `budget ${Math.max(0, sheetsToExport.budgets.rowCount - 2)}, `
    + `exchange rates ${Math.max(0, sheetsToExport.exchangeRates.rowCount - 1)}.`);
  console.log(`Export SHA-256: ${sha256}`);
  console.log('The archive contains private finance data. It is read-only with respect to Google Sheets.');
});

function columnCountName(columnNumber) {
  let value = columnNumber;
  let result = '';
  while (value > 0) {
    value -= 1;
    result = String.fromCharCode(65 + (value % 26)) + result;
    value = Math.floor(value / 26);
  }
  return result;
}
