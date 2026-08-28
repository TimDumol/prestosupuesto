import {
  columnName,
  finish,
  headersFor,
  option,
  parseArgs,
  quoteSheetTitle,
  runtime,
  sheetOption,
} from './lib.mjs';

function isFormula(value) {
  return typeof value === 'string' && value.startsWith('=');
}

function populated(value) {
  return value !== undefined && value !== null && value !== '';
}

function role(value) {
  if (!populated(value)) return 'blank';
  if (value === 'Expense') return 'Expense';
  if (value === 'Income') return 'Income';
  return 'value';
}

function valueMode(value) {
  if (!populated(value)) return 'blank';
  if (isFormula(value)) return 'formula';
  return 'literal';
}

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const { sheets, spreadsheetId } = await runtime(args);
  const sheet = sheetOption(args);
  const headerRow = Number(option(args, 'header-row', 1));
  const headers = await headersFor(sheets, spreadsheetId, sheet, headerRow);
  const endColumn = columnName(headers.length);
  const range = `${quoteSheetTitle(sheet)}!A${headerRow}:${endColumn}`;

  const [formulaResponse, rawResponse, accountResponse] = await Promise.all([
    sheets.spreadsheets.values.get({
      spreadsheetId,
      range,
      majorDimension: 'ROWS',
      valueRenderOption: 'FORMULA',
    }),
    sheets.spreadsheets.values.get({
      spreadsheetId,
      range,
      majorDimension: 'ROWS',
      valueRenderOption: 'UNFORMATTED_VALUE',
    }),
    sheets.spreadsheets.values.get({
      spreadsheetId,
      range: 'Accounts!A2:D',
      majorDimension: 'ROWS',
      valueRenderOption: 'UNFORMATTED_VALUE',
    }),
  ]);
  const accountCurrencies = new Map((accountResponse.data.values ?? [])
    .filter((row) => row[0])
    .map((row) => [String(row[0]), String(row[3] ?? '')]));
  const formulaRows = formulaResponse.data.values ?? [];
  const rawRows = rawResponse.data.values ?? [];
  const rowCount = Math.max(formulaRows.length, rawRows.length);
  const dataStart = headerRow;
  const inputEndIndex = Math.min(8, headers.length - 1);
  const recordRows = [];
  const preparedEmptyRows = [];
  const signatures = new Map();
  const archetypes = new Map([
    ['expense', []],
    ['income', []],
    ['account-transfer', []],
    ['cross-currency-transfer', []],
    ['budget-reallocation', []],
    ['unclassified', []],
  ]);

  for (let index = dataStart; index < rowCount; index += 1) {
    const formulaRow = formulaRows[index] ?? [];
    const rawRow = rawRows[index] ?? [];
    const hasInput = rawRow.slice(0, inputEndIndex + 1).some(populated);
    const hasDerivedFormula = formulaRow.some(isFormula);
    if (hasInput) {
      recordRows.push(index + 1);
      const signature = [
        `F:${role(rawRow[5])}`,
        `G:${role(rawRow[6])}`,
        `H:${role(rawRow[7])}`,
        `I:${role(rawRow[8])}`,
        `C:${valueMode(formulaRow[2])}`,
        `D:${valueMode(formulaRow[3])}`,
        `E:${valueMode(formulaRow[4])}`,
      ].join(' | ');
      signatures.set(signature, (signatures.get(signature) ?? 0) + 1);
      let archetype = 'unclassified';
      if (rawRow[6] === 'Expense' && rawRow[8] === 'Expense') archetype = 'expense';
      else if (rawRow[5] === 'Income' && rawRow[7] === 'Income') archetype = 'income';
      else if (rawRow[5] === 'Balance Transfer' && rawRow[6] === 'Balance Transfer') {
        const fromCurrency = accountCurrencies.get(String(rawRow[7] ?? ''));
        const toCurrency = accountCurrencies.get(String(rawRow[8] ?? ''));
        archetype = fromCurrency && toCurrency && fromCurrency !== toCurrency
          ? 'cross-currency-transfer'
          : 'account-transfer';
      } else if (rawRow[7] === 'Reallocation' && rawRow[8] === 'Reallocation') {
        archetype = 'budget-reallocation';
      }
      archetypes.get(archetype).push(index + 1);
    } else if (hasDerivedFormula) {
      preparedEmptyRows.push(index + 1);
    }
  }

  const formulaCoverage = headers.map((header, columnIndex) => {
    const rows = [];
    for (let index = dataStart; index < formulaRows.length; index += 1) {
      if (isFormula(formulaRows[index]?.[columnIndex])) rows.push(index + 1);
    }
    return {
      Column: columnName(columnIndex + 1),
      Header: header,
      FormulaCount: rows.length,
      FirstFormulaRow: rows[0] ?? '',
      LastFormulaRow: rows.at(-1) ?? '',
    };
  });

  const nextPreparedRow = preparedEmptyRows.find((rowNumber) => {
    const lastRecord = recordRows.at(-1) ?? headerRow;
    return rowNumber > lastRecord;
  });
  console.log(`Spreadsheet: ${spreadsheetId}`);
  console.log(`Ledger: ${sheet} (${range})`);
  console.log(`Rows containing input data: ${recordRows.length}`);
  console.log(`Last input row: ${recordRows.at(-1) ?? 'none'}`);
  console.log(`Prepared empty rows containing formulas: ${preparedEmptyRows.length}`);
  console.log(`Next prepared row after the ledger: ${nextPreparedRow ?? 'none'}`);
  if (!args.summary) {
    console.log('Formula coverage by column:');
    console.table(formulaCoverage);
    console.log('Transaction shapes (values are not printed):');
    console.table([...signatures.entries()]
      .map(([Signature, Count]) => ({ Count, Signature }))
      .sort((left, right) => right.Count - left.Count));
  }
  console.log('Recognized transaction archetypes:');
  console.table([...archetypes.entries()].map(([Archetype, rows]) => ({
    Archetype,
    Count: rows.length,
    ExampleRows: rows.slice(-5).join(', '),
  })));

  if (nextPreparedRow && !args.summary) {
    const template = formulaRows[nextPreparedRow - 1] ?? [];
    console.log(`Formula template in row ${nextPreparedRow}:`);
    console.table(template.map((value, index) => ({
      Column: columnName(index + 1),
      Header: headers[index],
      Mode: valueMode(value),
      Formula: isFormula(value) ? value : '',
    })).filter((item) => item.Mode === 'formula'));
  }
});
