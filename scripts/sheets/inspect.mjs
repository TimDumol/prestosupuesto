import {
  finish,
  headersFor,
  option,
  parseArgs,
  runtime,
  sheetOption,
} from './lib.mjs';

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const { sheets, spreadsheetId } = await runtime(args);
  const sheet = sheetOption(args);
  const headerRow = Number(option(args, 'header-row', 1));
  const headers = await headersFor(sheets, spreadsheetId, sheet, headerRow);

  console.log(`Sheet: ${sheet}`);
  console.log(`Header row: ${headerRow}`);
  console.table(headers.map((Header, index) => ({ Position: index + 1, Header })));
});
