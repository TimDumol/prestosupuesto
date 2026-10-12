import { appendFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import {
  columnName,
  extractSpreadsheetId,
  finish,
  headersFor,
  option,
  parseArgs,
  quoteSheetTitle,
  runtime,
} from './lib.mjs';
import {
  buildTransaction,
  formulaTemplateProblems,
  TRANSACTION_FORMULA_COLUMNS,
  TRANSACTION_FULL_HEADERS,
  TRANSACTION_HEADERS,
  TRANSACTION_INPUT_COLUMNS,
  TRANSACTION_METADATA_HEADERS,
} from './finance-schema.mjs';

function populated(value) {
  return value !== undefined && value !== null && value !== '';
}

function isFormula(value) {
  return typeof value === 'string' && value.startsWith('=');
}

function required(args, name) {
  const value = option(args, name);
  if (value === undefined || value === true || value === '') throw new Error(`Pass --${name}.`);
  return String(value);
}

function transactionInput(args) {
  const kind = required(args, 'kind');
  return buildTransaction({
    kind,
    date: String(option(args, 'date', new Date().toISOString().slice(0, 10))),
    amount: required(args, 'amount'),
    description: option(args, 'description', `${kind} round-trip`),
    fromCategory: option(args, 'from-category'),
    toCategory: option(args, 'to-category'),
    fromAccount: option(args, 'from-account'),
    toAccount: option(args, 'to-account'),
    toAmount: option(args, 'to-amount'),
    currency: option(args, 'currency'),
  });
}

async function allowedValues(sheets, spreadsheetId) {
  const [categoryResponse, accountResponse] = await Promise.all([
    sheets.spreadsheets.values.get({ spreadsheetId, range: "'Expenses v3'!A3:A" }),
    sheets.spreadsheets.values.get({ spreadsheetId, range: 'Accounts!A2:D' }),
  ]);
  return {
    categories: new Set((categoryResponse.data.values ?? []).flat().map(String)),
    accounts: new Set((accountResponse.data.values ?? []).map((row) => String(row[0] ?? '')).filter(Boolean)),
    accountCurrencies: new Map((accountResponse.data.values ?? [])
      .filter((row) => row[0])
      .map((row) => [String(row[0]), String(row[3] ?? '')])),
  };
}

function validateReferences(input, allowed) {
  for (const column of [5, 6]) {
    const value = input.values.get(column);
    if (!allowed.categories.has(value)) throw new Error(`Unknown category "${value}".`);
  }
  for (const column of [7, 8]) {
    const value = input.values.get(column);
    if (!allowed.accounts.has(value)) throw new Error(`Unknown account "${value}".`);
  }
  if (input.kind === 'transfer') {
    const fromCurrency = allowed.accountCurrencies.get(input.values.get(7));
    const toCurrency = allowed.accountCurrencies.get(input.values.get(8));
    if (!fromCurrency || !toCurrency) throw new Error('Both transfer accounts must have configured currencies.');
    if (fromCurrency !== toCurrency && !input.values.has(4)) {
      throw new Error(`Transfer ${fromCurrency} → ${toCurrency} requires --to-amount.`);
    }
    if (fromCurrency === toCurrency && input.values.has(4)) {
      throw new Error('Do not pass --to-amount for a same-currency transfer.');
    }
  }
}

async function findPreparedRow(sheets, spreadsheetId, sheet, headerRow, columnCount) {
  const endColumn = columnName(columnCount);
  const range = `${quoteSheetTitle(sheet)}!A${headerRow + 1}:${endColumn}`;
  const [formulaResponse, rawResponse] = await Promise.all([
    sheets.spreadsheets.values.get({
      spreadsheetId, range, majorDimension: 'ROWS', valueRenderOption: 'FORMULA',
    }),
    sheets.spreadsheets.values.get({
      spreadsheetId, range, majorDimension: 'ROWS', valueRenderOption: 'UNFORMATTED_VALUE',
    }),
  ]);
  const formulaRows = formulaResponse.data.values ?? [];
  const rawRows = rawResponse.data.values ?? [];
  const rowCount = Math.max(formulaRows.length, rawRows.length);
  let lastInputIndex = -1;
  for (let index = 0; index < rowCount; index += 1) {
    if ((rawRows[index] ?? []).slice(0, 9).some(populated)) lastInputIndex = index;
  }
  const candidateIndex = lastInputIndex + 1;
  const rowNumber = headerRow + 1 + candidateIndex;
  const formulaRow = formulaRows[candidateIndex] ?? [];
  const rawRow = rawRows[candidateIndex] ?? [];
  if (TRANSACTION_INPUT_COLUMNS.some((column) => populated(rawRow[column]))) {
    throw new Error(`Candidate row ${rowNumber} is not empty in its input columns.`);
  }
  const missingFormulas = TRANSACTION_FORMULA_COLUMNS.filter((column) => !isFormula(formulaRow[column]));
  if (missingFormulas.length) {
    throw new Error(
      `Candidate row ${rowNumber} is missing formulas in: ${missingFormulas.map((column) => columnName(column + 1)).join(', ')}.`,
    );
  }
  return { rowNumber, formulaRow };
}

async function validationCount(sheets, spreadsheetId, sheet, rowNumber) {
  const response = await sheets.spreadsheets.get({
    spreadsheetId,
    includeGridData: true,
    ranges: [`${quoteSheetTitle(sheet)}!A${rowNumber}:I${rowNumber}`],
    fields: 'sheets(data(rowData(values(dataValidation))))',
  });
  const values = response.data.sheets?.[0]?.data?.[0]?.rowData?.[0]?.values ?? [];
  return values.filter((cell) => cell.dataValidation).length;
}

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const testSpreadsheetId = extractSpreadsheetId(process.env.GOOGLE_SHEETS_TEST_SPREADSHEET_ID);
  const productionId = process.env.GOOGLE_SHEETS_SPREADSHEET_ID
    ? extractSpreadsheetId(process.env.GOOGLE_SHEETS_SPREADSHEET_ID)
    : null;
  if (productionId && productionId === testSpreadsheetId) {
    throw new Error('Test and production spreadsheet IDs must be different.');
  }
  args.spreadsheet = testSpreadsheetId;

  const input = transactionInput(args);
  const { sheets, spreadsheetId } = await runtime(args);
  const sheet = String(option(args, 'sheet', 'Transactions'));
  const headerRow = Number(option(args, 'header-row', 1));
  const headers = await headersFor(sheets, spreadsheetId, sheet, headerRow);
  if (JSON.stringify(headers.slice(0, TRANSACTION_HEADERS.length)) !== JSON.stringify(TRANSACTION_HEADERS)) {
    throw new Error('Transactions header fingerprint has changed; refusing the test write.');
  }
  const metadataHeaders = headers.slice(
    TRANSACTION_HEADERS.length,
    TRANSACTION_HEADERS.length + TRANSACTION_METADATA_HEADERS.length,
  );
  const hasMetadata = JSON.stringify(metadataHeaders) === JSON.stringify(TRANSACTION_METADATA_HEADERS);
  if (headers.length > TRANSACTION_HEADERS.length && !hasMetadata) {
    throw new Error('Transaction metadata headers are partial or differ from their contract.');
  }
  validateReferences(input, await allowedValues(sheets, spreadsheetId));

  const prepared = await findPreparedRow(
    sheets,
    spreadsheetId,
    sheet,
    headerRow,
    TRANSACTION_HEADERS.length,
  );
  const formulaProblems = formulaTemplateProblems(prepared.formulaRow, prepared.rowNumber);
  if (formulaProblems.length) {
    throw new Error(`Prepared-row formula contract changed: ${formulaProblems.join('; ')}.`);
  }
  const expectedRow = option(args, 'expected-row');
  if (expectedRow && Number(expectedRow) !== prepared.rowNumber) {
    throw new Error(`Expected row ${expectedRow}, but the next prepared row is ${prepared.rowNumber}.`);
  }
  const validations = await validationCount(sheets, spreadsheetId, sheet, prepared.rowNumber);
  if (validations < 5) {
    throw new Error(`Prepared row ${prepared.rowNumber} has only ${validations} validation rule(s); expected 5.`);
  }

  console.log(`Test workbook: ${spreadsheetId}`);
  console.log(`Prepared row: ${prepared.rowNumber}`);
  console.log(
    `Kind: ${input.kind}; date: ${input.date}; amount: ${input.amount}`
      + `${input.toAmount === null ? '' : `; to amount: ${input.toAmount}`}`
      + `${input.values.has(3) ? `; transaction currency: ${input.values.get(3)}` : ''}`
      + `; metadata: ${hasMetadata ? 'enabled' : 'not installed'}; validations: ${validations}`,
  );
  const createdAt = new Date().toISOString();
  const createdBy = String(option(args, 'created-by', process.env.GOOGLE_SHEETS_CREATED_BY ?? 'local_test'));
  const writeValues = new Map(input.values);
  if (hasMetadata) {
    writeValues.set(17, input.testId);
    writeValues.set(18, createdAt);
    writeValues.set(19, createdBy);
  }
  console.table([...writeValues.entries()].map(([column, value]) => ({
    Cell: `${columnName(column + 1)}${prepared.rowNumber}`,
    Header: TRANSACTION_FULL_HEADERS[column] ?? headers[column],
    Value: column === 1 ? '[API TEST description]' : value,
  })));
  if (!args.commit) {
    console.log('Dry run only. Add --commit to write this transaction to the configured test workbook.');
    return;
  }

  await sheets.spreadsheets.values.batchUpdate({
    spreadsheetId,
    requestBody: {
      valueInputOption: 'RAW',
      data: [...writeValues.entries()].map(([column, value]) => ({
        range: `${quoteSheetTitle(sheet)}!${columnName(column + 1)}${prepared.rowNumber}`,
        majorDimension: 'ROWS',
        values: [[value]],
      })),
    },
  });

  const rowRange = `${quoteSheetTitle(sheet)}!A${prepared.rowNumber}:Q${prepared.rowNumber}`;
  const [formulaResult, rawResult, formattedResult] = await Promise.all([
    sheets.spreadsheets.values.get({ spreadsheetId, range: rowRange, valueRenderOption: 'FORMULA' }),
    sheets.spreadsheets.values.get({ spreadsheetId, range: rowRange, valueRenderOption: 'UNFORMATTED_VALUE' }),
    sheets.spreadsheets.values.get({ spreadsheetId, range: rowRange, valueRenderOption: 'FORMATTED_VALUE' }),
  ]);
  const formulaRow = formulaResult.data.values?.[0] ?? [];
  const rawRow = rawResult.data.values?.[0] ?? [];
  const formattedRow = formattedResult.data.values?.[0] ?? [];
  const intentionallyReplacedFormulaColumns = new Set(
    TRANSACTION_FORMULA_COLUMNS.filter((column) => input.values.has(column)),
  );
  const changedFormulaColumns = TRANSACTION_FORMULA_COLUMNS.filter(
    (column) => !intentionallyReplacedFormulaColumns.has(column),
  ).filter(
    (column) => formulaRow[column] !== prepared.formulaRow[column],
  );
  if (changedFormulaColumns.length) {
    throw new Error(`Formula integrity check failed in ${changedFormulaColumns.map((column) => columnName(column + 1)).join(', ')}.`);
  }

  const auditPath = path.resolve(process.cwd(), '.google/test-writes.jsonl');
  await mkdir(path.dirname(auditPath), { recursive: true });
  await appendFile(auditPath, `${JSON.stringify({
    at: new Date().toISOString(), spreadsheetId, sheet, row: prepared.rowNumber,
    testId: input.testId, kind: input.kind, date: input.date, amount: input.amount,
    toAmount: input.toAmount, currencyOverride: input.values.get(3) ?? null,
    createdAt, createdBy,
  })}\n`, 'utf8');

  console.log(`Committed test transaction to ${sheet} row ${prepared.rowNumber}.`);
  console.log('Formula integrity: passed.');
  console.table(headers.map((Header, index) => ({
    Column: columnName(index + 1),
    Header,
    Raw: index === 1 ? '[API TEST description]' : rawRow[index] ?? '',
    Displayed: index === 1 ? '[API TEST description]' : formattedRow[index] ?? '',
    Formula: isFormula(formulaRow[index]) ? formulaRow[index] : '',
  })));
});
