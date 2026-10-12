import {
  finish,
  option,
  parseArgs,
  quoteSheetTitle,
  runtime,
} from './lib.mjs';

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const { sheets, spreadsheetId } = await runtime(args);
  const transactionSheet = process.env.GOOGLE_SHEETS_TRANSACTIONS_SHEET ?? 'Transactions';
  const budgetSheet = process.env.GOOGLE_SHEETS_BUDGETS_SHEET ?? 'Budgets';
  const ranges = [
    String(option(args, 'transactions-range', `${quoteSheetTitle(transactionSheet)}!A:Z`)),
    String(option(args, 'budgets-range', `${quoteSheetTitle(budgetSheet)}!A:Z`)),
  ];
  const response = await sheets.spreadsheets.values.batchGet({
    spreadsheetId,
    ranges,
    majorDimension: 'ROWS',
    valueRenderOption: args.raw ? 'UNFORMATTED_VALUE' : 'FORMATTED_VALUE',
  });

  const result = Object.fromEntries((response.data.valueRanges ?? []).map((item, index) => [
    item.range ?? ranges[index],
    item.values ?? [],
  ]));
  console.log(JSON.stringify({
    spreadsheetId,
    retrievedAt: new Date().toISOString(),
    ranges: result,
  }, null, 2));
});
