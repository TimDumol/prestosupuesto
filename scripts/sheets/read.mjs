import {
  finish,
  option,
  parseArgs,
  quoteSheetTitle,
  runtime,
  sheetOption,
} from './lib.mjs';

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const { sheets, spreadsheetId } = await runtime(args);
  const sheet = sheetOption(args);
  const range = String(option(args, 'range', `${quoteSheetTitle(sheet)}!A1:Z20`));
  const response = await sheets.spreadsheets.values.get({
    spreadsheetId,
    range,
    majorDimension: 'ROWS',
    valueRenderOption: args.raw ? 'UNFORMATTED_VALUE' : 'FORMATTED_VALUE',
  });
  const values = response.data.values ?? [];

  if (args.json) {
    console.log(JSON.stringify({ range: response.data.range, values }, null, 2));
    return;
  }
  console.log(`Range: ${response.data.range ?? range}`);
  if (!values.length) console.log('No values found.');
  else console.table(values);
});
