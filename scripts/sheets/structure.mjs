import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import {
  columnName,
  finish,
  option,
  optionList,
  parseArgs,
  quoteSheetTitle,
  runtime,
} from './lib.mjs';

function cellReference(row, column) {
  return `${columnName(column + 1)}${row + 1}`;
}

function hasCellStructure(cell) {
  return cell.userEnteredValue !== undefined
    || cell.effectiveValue !== undefined
    || cell.formattedValue !== undefined
    || cell.note !== undefined
    || cell.dataValidation !== undefined
    || cell.userEnteredFormat?.numberFormat !== undefined
    || cell.effectiveFormat?.numberFormat !== undefined;
}

function compactGridData(sheet) {
  const cells = [];
  for (const grid of sheet.data ?? []) {
    const startRow = grid.startRow ?? 0;
    const startColumn = grid.startColumn ?? 0;
    (grid.rowData ?? []).forEach((row, rowOffset) => {
      (row.values ?? []).forEach((cell, columnOffset) => {
        if (!hasCellStructure(cell)) return;
        cells.push({
          cell: cellReference(startRow + rowOffset, startColumn + columnOffset),
          userEnteredValue: cell.userEnteredValue,
          effectiveValue: cell.effectiveValue,
          formattedValue: cell.formattedValue,
          numberFormat: cell.userEnteredFormat?.numberFormat,
          effectiveNumberFormat: cell.effectiveFormat?.numberFormat,
          dataValidation: cell.dataValidation,
          note: cell.note,
        });
      });
    });
  }
  return cells;
}

function compactMetadata(workbook) {
  return {
    spreadsheetId: workbook.spreadsheetId,
    spreadsheetUrl: workbook.spreadsheetUrl,
    properties: workbook.properties,
    namedRanges: workbook.namedRanges ?? [],
    developerMetadata: workbook.developerMetadata ?? [],
    sheets: (workbook.sheets ?? []).map((sheet) => ({
      properties: sheet.properties,
      merges: sheet.merges ?? [],
      protectedRanges: sheet.protectedRanges ?? [],
      conditionalFormats: sheet.conditionalFormats ?? [],
      basicFilter: sheet.basicFilter,
      filterViews: sheet.filterViews ?? [],
      tables: sheet.tables ?? [],
      charts: (sheet.charts ?? []).map((chart) => ({
        chartId: chart.chartId,
        position: chart.position,
        title: chart.spec?.title,
        chartType: Object.keys(chart.spec ?? {}).find((key) => key.endsWith('Chart')),
      })),
    })),
  };
}

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const { sheets, spreadsheetId } = await runtime(args);
  const rowLimit = Number(option(args, 'rows', 250));
  const columnLimit = Number(option(args, 'columns', 52));
  if (!Number.isInteger(rowLimit) || rowLimit < 1) throw new Error('--rows must be a positive integer.');
  if (!Number.isInteger(columnLimit) || columnLimit < 1 || columnLimit > 702) {
    throw new Error('--columns must be an integer between 1 and 702.');
  }

  const metadataResponse = await sheets.spreadsheets.get({
    spreadsheetId,
    includeGridData: false,
  });
  const metadata = compactMetadata(metadataResponse.data);
  const requestedRanges = optionList(args, 'range').map(String);
  const ranges = requestedRanges.length
    ? requestedRanges
    : metadata.sheets
      .filter((sheet) => sheet.properties?.sheetType === 'GRID')
      .map((sheet) => `${quoteSheetTitle(sheet.properties.title)}!A1:${columnName(columnLimit)}${rowLimit}`);

  const gridResponse = await sheets.spreadsheets.get({
    spreadsheetId,
    includeGridData: true,
    ranges,
    fields: 'sheets(properties(sheetId,title),data(startRow,startColumn,rowData(values('
      + 'userEnteredValue,effectiveValue,formattedValue,note,dataValidation,'
      + 'userEnteredFormat(numberFormat),effectiveFormat(numberFormat)))))',
  });

  const gridsBySheetId = new Map((gridResponse.data.sheets ?? []).map((sheet) => [
    sheet.properties?.sheetId,
    compactGridData(sheet),
  ]));
  const report = {
    generatedAt: new Date().toISOString(),
    inspectedRanges: ranges,
    ...metadata,
    sheets: metadata.sheets.map((sheet) => ({
      ...sheet,
      inspectedCells: gridsBySheetId.get(sheet.properties?.sheetId) ?? [],
    })),
  };

  const output = path.resolve(
    process.cwd(),
    String(option(args, 'output', '.google/workbook-structure.json')),
  );
  await mkdir(path.dirname(output), { recursive: true });
  await writeFile(output, JSON.stringify(report, null, 2), 'utf8');

  const formulaCount = report.sheets.reduce(
    (count, sheet) => count + sheet.inspectedCells.filter((cell) => cell.userEnteredValue?.formulaValue).length,
    0,
  );
  const validationCount = report.sheets.reduce(
    (count, sheet) => count + sheet.inspectedCells.filter((cell) => cell.dataValidation).length,
    0,
  );
  console.log(`Workbook structure saved to ${output}`);
  console.log(`Inspected ${ranges.length} range(s), ${formulaCount} formula cell(s), ${validationCount} validated cell(s).`);
  console.log('The report can contain personal financial data and is intentionally stored under .google/.');
});
