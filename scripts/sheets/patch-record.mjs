import {
  columnName,
  finish,
  headersFor,
  loadRecord,
  normalizeHeader,
  option,
  optionList,
  assignments,
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
  const keyColumn = option(args, 'key-column');
  const key = option(args, 'key');
  if (!keyColumn || key === undefined) throw new Error('Pass both --key-column and --key.');

  const patch = await loadRecord(args);
  if (!Object.keys(patch).length) throw new Error('Pass --data patch.json or one or more --set Header=Value arguments.');

  const headers = await headersFor(sheets, spreadsheetId, sheet, headerRow);
  if (JSON.stringify(headers) === JSON.stringify(TRANSACTION_HEADERS)) {
    throw new Error(
      'Generic patch is disabled for the current Transactions ledger because it has no stable ID column.',
    );
  }
  const keyIndex = headers.findIndex((header) => normalizeHeader(header) === normalizeHeader(keyColumn));
  if (keyIndex === -1) throw new Error(`Key column "${keyColumn}" was not found.`);

  const endColumn = columnName(headers.length);
  const dataRange = `${quoteSheetTitle(sheet)}!A${headerRow + 1}:${endColumn}`;
  const response = await sheets.spreadsheets.values.get({
    spreadsheetId,
    range: dataRange,
    majorDimension: 'ROWS',
    valueRenderOption: 'UNFORMATTED_VALUE',
  });
  const rows = response.data.values ?? [];
  const matches = rows
    .map((row, index) => ({ row, rowNumber: headerRow + index + 1 }))
    .filter(({ row }) => String(row[keyIndex] ?? '') === String(key));
  if (matches.length !== 1) {
    throw new Error(`Expected one row where ${headers[keyIndex]}=${key}; found ${matches.length}.`);
  }

  const { row, rowNumber } = matches[0];
  const resolved = resolveRecord(headers, patch);
  if (resolved.has(keyIndex)) throw new Error('The key column cannot be changed by this command.');

  const formulaResponse = await sheets.spreadsheets.values.get({
    spreadsheetId,
    range: `${quoteSheetTitle(sheet)}!A${rowNumber}:${endColumn}${rowNumber}`,
    majorDimension: 'ROWS',
    valueRenderOption: 'FORMULA',
  });
  const formulaRow = formulaResponse.data.values?.[0] ?? [];
  const overwrittenFormulaHeaders = [...resolved.keys()]
    .filter((index) => typeof formulaRow[index] === 'string' && formulaRow[index].startsWith('='))
    .map((index) => headers[index]);
  if (overwrittenFormulaHeaders.length && !args['allow-formula-overwrite']) {
    throw new Error(
      `Refusing to overwrite formula cell(s): ${overwrittenFormulaHeaders.join(', ')}. `
      + 'Pass --allow-formula-overwrite only if replacing those formulas is intentional.',
    );
  }

  const expected = resolveRecord(headers, assignments(optionList(args, 'expect')));
  for (const [index, item] of expected) {
    if (String(row[index] ?? '') !== String(item.value ?? '')) {
      throw new Error(
        `Optimistic check failed for ${item.header}: expected "${item.value}", found "${row[index] ?? ''}".`,
      );
    }
  }

  console.log(`Patch ${sheet} row ${rowNumber}:`);
  printResolved(resolved, row);
  if (!args.commit) {
    console.log('Dry run only. Add --commit to update these cells.');
    return;
  }

  await sheets.spreadsheets.values.batchUpdate({
    spreadsheetId,
    requestBody: {
      valueInputOption: args.raw ? 'RAW' : 'USER_ENTERED',
      data: [...resolved.entries()].map(([index, item]) => ({
        range: `${quoteSheetTitle(sheet)}!${columnName(index + 1)}${rowNumber}`,
        majorDimension: 'ROWS',
        values: [[item.value]],
      })),
    },
  });
  console.log(`Committed ${resolved.size} cell update(s) on row ${rowNumber}.`);
});
