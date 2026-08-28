import {
  extractSpreadsheetId,
  finish,
  parseArgs,
  runtime,
} from './lib.mjs';
import {
  TRANSACTION_FULL_HEADERS,
  TRANSACTION_HEADERS,
  TRANSACTION_METADATA_HEADERS,
} from './finance-schema.mjs';

function headerMatches(actual, expected) {
  return JSON.stringify(actual) === JSON.stringify(expected);
}

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const spreadsheetId = extractSpreadsheetId(process.env.GOOGLE_SHEETS_SPREADSHEET_ID);
  const testSpreadsheetId = process.env.GOOGLE_SHEETS_TEST_SPREADSHEET_ID
    ? extractSpreadsheetId(process.env.GOOGLE_SHEETS_TEST_SPREADSHEET_ID)
    : null;
  if (testSpreadsheetId && spreadsheetId === testSpreadsheetId) {
    throw new Error('Live and test spreadsheet IDs must be different.');
  }
  args.spreadsheet = spreadsheetId;
  const { sheets } = await runtime(args);
  const metadataResponse = await sheets.spreadsheets.get({
    spreadsheetId,
    includeGridData: false,
    fields: 'sheets(properties(sheetId,title))',
  });
  const transactionSheet = metadataResponse.data.sheets?.find(
    (sheet) => sheet.properties?.title === 'Transactions',
  );
  if (transactionSheet?.properties?.sheetId === undefined) {
    throw new Error('Transactions sheet was not found.');
  }

  const response = await sheets.spreadsheets.values.get({
    spreadsheetId,
    range: "'Transactions'!A1:T1",
    majorDimension: 'ROWS',
    valueRenderOption: 'UNFORMATTED_VALUE',
  });
  const transactionHeader = response.data.values?.[0] ?? [];
  if (!headerMatches(transactionHeader.slice(0, TRANSACTION_HEADERS.length), TRANSACTION_HEADERS)) {
    throw new Error('Transactions A:Q header fingerprint changed; refusing migration.');
  }
  const existingMetadata = transactionHeader.slice(17, 20);
  if (existingMetadata.some((value) => value !== '')
      && !headerMatches(existingMetadata, TRANSACTION_METADATA_HEADERS)) {
    throw new Error('Transactions R:T is not blank and does not match the metadata contract.');
  }
  const metadataInstalled = headerMatches(transactionHeader, TRANSACTION_FULL_HEADERS);
  console.log(`Live workbook: ${spreadsheetId}`);
  console.log(`Transactions metadata: ${metadataInstalled ? 'already installed' : 'will add R:T and hide them'}`);
  if (!args.commit) {
    console.log('Dry run only. Add --commit to install live transaction metadata.');
    return;
  }

  if (!metadataInstalled) {
    await sheets.spreadsheets.values.update({
      spreadsheetId,
      range: "'Transactions'!R1:T1",
      valueInputOption: 'RAW',
      requestBody: { values: [TRANSACTION_METADATA_HEADERS] },
    });
  }
  await sheets.spreadsheets.batchUpdate({
    spreadsheetId,
    requestBody: {
      requests: [{
        updateDimensionProperties: {
          range: {
            sheetId: transactionSheet.properties.sheetId,
            dimension: 'COLUMNS',
            startIndex: 17,
            endIndex: 20,
          },
          properties: { hiddenByUser: true },
          fields: 'hiddenByUser',
        },
      }],
    },
  });

  const verify = await sheets.spreadsheets.get({
    spreadsheetId,
    includeGridData: true,
    ranges: ["'Transactions'!R1:T1"],
    fields: 'sheets(data(startColumn,columnMetadata(hiddenByUser),rowData(values(userEnteredValue))))',
  });
  const data = verify.data.sheets?.[0]?.data?.[0];
  const hiddenColumns = data?.columnMetadata ?? [];
  const verifiedHeaders = data?.rowData?.[0]?.values
    ?.map((cell) => cell.userEnteredValue?.stringValue ?? '') ?? [];
  if (!headerMatches(verifiedHeaders, TRANSACTION_METADATA_HEADERS)
      || hiddenColumns.length < 3
      || hiddenColumns.some((column) => !column.hiddenByUser)) {
    throw new Error('Live metadata migration verification failed.');
  }
  console.log('Live schema migration committed and verified. Transactions R:T are hidden metadata columns.');
});