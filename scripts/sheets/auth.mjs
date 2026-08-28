import { finish, parseArgs, runtime } from './lib.mjs';

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const { sheets, spreadsheetId, mode } = await runtime(args);
  const response = await sheets.spreadsheets.get({
    spreadsheetId,
    fields: 'properties.title,sheets.properties(sheetId,title,gridProperties)',
  });

  console.log(`Authenticated via ${mode}.`);
  console.log(`Spreadsheet: ${response.data.properties?.title} (${spreadsheetId})`);
  console.table((response.data.sheets ?? []).map(({ properties }) => ({
    ID: properties?.sheetId,
    Sheet: properties?.title,
    Rows: properties?.gridProperties?.rowCount,
    Columns: properties?.gridProperties?.columnCount,
  })));
});
