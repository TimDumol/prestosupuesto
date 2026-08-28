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
  const testSpreadsheetId = extractSpreadsheetId(process.env.GOOGLE_SHEETS_TEST_SPREADSHEET_ID);
  const productionId = process.env.GOOGLE_SHEETS_SPREADSHEET_ID
    ? extractSpreadsheetId(process.env.GOOGLE_SHEETS_SPREADSHEET_ID)
    : null;
  if (productionId && productionId === testSpreadsheetId) {
    throw new Error('Test and production spreadsheet IDs must be different.');
  }
  args.spreadsheet = testSpreadsheetId;
  const { sheets, spreadsheetId } = await runtime(args);

  const metadataResponse = await sheets.spreadsheets.get({
    spreadsheetId,
    includeGridData: false,
    fields: 'sheets(properties(sheetId,title,gridProperties(rowCount,columnCount)))',
  });
  const transactionSheet = metadataResponse.data.sheets?.find(
    (sheet) => sheet.properties?.title === 'Transactions',
  );
  const accountsSheet = metadataResponse.data.sheets?.find(
    (sheet) => sheet.properties?.title === 'Accounts',
  );
  if (transactionSheet?.properties?.sheetId === undefined
      || accountsSheet?.properties?.sheetId === undefined) {
    throw new Error('Transactions or Accounts sheet was not found.');
  }

  const response = await sheets.spreadsheets.values.batchGet({
    spreadsheetId,
    ranges: ["'Transactions'!A1:T1", "'Accounts'!A1:R"],
    majorDimension: 'ROWS',
    valueRenderOption: 'UNFORMATTED_VALUE',
  });
  const transactionHeader = response.data.valueRanges?.[0]?.values?.[0] ?? [];
  const accountRows = response.data.valueRanges?.[1]?.values ?? [];
  if (!headerMatches(transactionHeader.slice(0, TRANSACTION_HEADERS.length), TRANSACTION_HEADERS)) {
    throw new Error('Transactions A:Q header fingerprint changed; refusing migration.');
  }
  const existingMetadata = transactionHeader.slice(17, 20);
  if (existingMetadata.some((value) => value !== '')
      && !headerMatches(existingMetadata, TRANSACTION_METADATA_HEADERS)) {
    throw new Error('Transactions R:T is not blank and does not match the metadata contract.');
  }
  if (accountRows[0]?.[17] && accountRows[0][17] !== 'IsActive') {
    throw new Error('Accounts R1 is already in use; refusing migration.');
  }
  const populatedAccounts = accountRows.slice(1)
    .map((row, index) => ({ row, rowNumber: index + 2 }))
    .filter(({ row }) => row[0]);
  const metadataInstalled = headerMatches(transactionHeader, TRANSACTION_FULL_HEADERS);
  const accountActivityInstalled = accountRows[0]?.[17] === 'IsActive';

  console.log(`Test workbook: ${spreadsheetId}`);
  console.log(`Transactions metadata: ${metadataInstalled ? 'already installed' : 'will add R:T and hide them'}`);
  console.log(
    `Account activity: ${accountActivityInstalled ? 'already installed' : `will add R and initialize ${populatedAccounts.length} account row(s)`}`,
  );
  if (!args.commit) {
    console.log('Dry run only. Add --commit to migrate the configured test workbook.');
    return;
  }

  const valueUpdates = [];
  if (!metadataInstalled) {
    valueUpdates.push({
      range: "'Transactions'!R1:T1",
      majorDimension: 'ROWS',
      values: [TRANSACTION_METADATA_HEADERS],
    });
  }
  if (!accountActivityInstalled) {
    valueUpdates.push({ range: "'Accounts'!R1", values: [['IsActive']] });
    for (const { row, rowNumber } of populatedAccounts) {
      valueUpdates.push({ range: `'Accounts'!R${rowNumber}`, values: [[Boolean(row[2])]] });
    }
  }
  // Accounts P intersects existing merged warning cells. Remove only values
  // created there by the superseded first migration attempt.
  await sheets.spreadsheets.values.clear({
    spreadsheetId,
    range: `'Accounts'!P2:P${populatedAccounts.at(-1)?.rowNumber ?? 2}`,
  });
  if (valueUpdates.length) {
    await sheets.spreadsheets.values.batchUpdate({
      spreadsheetId,
      requestBody: { valueInputOption: 'RAW', data: valueUpdates },
    });
  }

  const requests = [{
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
  }];
  if (populatedAccounts.length) {
    requests.push({
      setDataValidation: {
        range: {
          sheetId: accountsSheet.properties.sheetId,
          startRowIndex: 1,
          endRowIndex: populatedAccounts.at(-1).rowNumber,
          startColumnIndex: 17,
          endColumnIndex: 18,
        },
        rule: {
          condition: { type: 'BOOLEAN' },
          strict: true,
          showCustomUi: true,
        },
      },
    });
  }
  requests.push({
    setDataValidation: {
      range: {
        sheetId: accountsSheet.properties.sheetId,
        startRowIndex: 1,
        endRowIndex: populatedAccounts.at(-1)?.rowNumber ?? 2,
        startColumnIndex: 15,
        endColumnIndex: 16,
      },
      rule: null,
    },
  });
  await sheets.spreadsheets.batchUpdate({ spreadsheetId, requestBody: { requests } });

  const verifyResponse = await sheets.spreadsheets.get({
    spreadsheetId,
    includeGridData: true,
    ranges: ["'Transactions'!R1:T1", "'Accounts'!R1:R"],
    fields: 'sheets(properties(sheetId,title),data(startColumn,columnMetadata(hiddenByUser),rowData(values(userEnteredValue,dataValidation))))',
  });
  const transactionVerify = verifyResponse.data.sheets?.find(
    (sheet) => sheet.properties?.title === 'Transactions',
  );
  const accountsVerify = verifyResponse.data.sheets?.find(
    (sheet) => sheet.properties?.title === 'Accounts',
  );
  const hiddenColumns = transactionVerify?.data?.[0]?.columnMetadata ?? [];
  const verifiedHeaders = transactionVerify?.data?.[0]?.rowData?.[0]?.values
    ?.map((cell) => cell.userEnteredValue?.stringValue ?? '') ?? [];
  const accountHeader = accountsVerify?.data?.[0]?.rowData?.[0]?.values?.[0]
    ?.userEnteredValue?.stringValue;
  if (!headerMatches(verifiedHeaders, TRANSACTION_METADATA_HEADERS)
      || hiddenColumns.length < 3
      || hiddenColumns.some((column) => !column.hiddenByUser)
      || accountHeader !== 'IsActive') {
    throw new Error('Migration verification failed.');
  }
  console.log('Test schema migration committed and verified. Transactions R:T are hidden; Accounts R is IsActive.');
});
