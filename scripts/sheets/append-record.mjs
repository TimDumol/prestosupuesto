import {
  columnName,
  finish,
  headersFor,
  loadRecord,
  option,
  parseArgs,
  printResolved,
  quoteSheetTitle,
  resolveRecord,
  runtime,
  sheetOption,
} from './lib.mjs';
import { TRANSACTION_HEADERS } from './finance-schema.mjs';

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const { sheets, spreadsheetId } = await runtime(args);
  const sheet = sheetOption(args);
  const headerRow = Number(option(args, 'header-row', 1));
  const record = await loadRecord(args);
  if (!Object.keys(record).length) throw new Error('Pass --data record.json or one or more --set Header=Value arguments.');

  const headers = await headersFor(sheets, spreadsheetId, sheet, headerRow);
  if (JSON.stringify(headers) === JSON.stringify(TRANSACTION_HEADERS)) {
    throw new Error(
      'Generic append is disabled for the formula-prepared Transactions ledger. '
      + 'Use sheets:test-transaction on the test copy; production will use the serialized gateway.',
    );
  }
  const resolved = resolveRecord(headers, record);
  const row = Array(headers.length).fill(null);
  for (const [index, item] of resolved) row[index] = item.value;

  const endColumn = columnName(headers.length);
  const formulaResponse = await sheets.spreadsheets.values.get({
    spreadsheetId,
    range: `${quoteSheetTitle(sheet)}!A${headerRow + 1}:${endColumn}`,
    majorDimension: 'ROWS',
    valueRenderOption: 'FORMULA',
  });
  const formulaColumns = new Map();
  for (const existingRow of formulaResponse.data.values ?? []) {
    existingRow.forEach((value, index) => {
      if (typeof value === 'string' && value.startsWith('=')) {
        formulaColumns.set(index, (formulaColumns.get(index) ?? 0) + 1);
      }
    });
  }
  const overwrittenFormulaHeaders = [...resolved.keys()]
    .filter((index) => formulaColumns.has(index))
    .map((index) => headers[index]);
  if (overwrittenFormulaHeaders.length && !args['allow-formula-overwrite']) {
    throw new Error(
      `Refusing to write formula-bearing column(s): ${overwrittenFormulaHeaders.join(', ')}. `
      + 'Remove those fields or pass --allow-formula-overwrite after reviewing the workbook structure.',
    );
  }

  console.log(`Append to ${sheet}:`);
  printResolved(resolved);
  if (formulaColumns.size) {
    console.warn(`Formula-bearing columns detected: ${[...formulaColumns.entries()]
      .map(([index, count]) => `${headers[index]} (${count})`)
      .join(', ')}`);
    console.warn('Appending does not guarantee that per-row formulas will be copied into the new row.');
  }
  if (!args.commit) {
    console.log('Dry run only. Add --commit to append this row.');
    return;
  }

  const response = await sheets.spreadsheets.values.append({
    spreadsheetId,
    range: `${quoteSheetTitle(sheet)}!A${headerRow}:${endColumn}`,
    valueInputOption: args.raw ? 'RAW' : 'USER_ENTERED',
    insertDataOption: 'INSERT_ROWS',
    includeValuesInResponse: true,
    requestBody: { majorDimension: 'ROWS', values: [row] },
  });
  console.log(`Committed: ${response.data.updates?.updatedRange ?? 'row appended'}`);
});
