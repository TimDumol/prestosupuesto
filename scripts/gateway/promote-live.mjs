import {
  extractSpreadsheetId,
  finish,
  parseArgs,
} from '../sheets/lib.mjs';
import {
  gatewayAuth,
  loadDeployment,
  runGateway,
  scriptService,
} from './lib.mjs';

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const spreadsheetId = extractSpreadsheetId(process.env.GOOGLE_SHEETS_SPREADSHEET_ID);
  const testSpreadsheetId = extractSpreadsheetId(process.env.GOOGLE_SHEETS_TEST_SPREADSHEET_ID);
  if (spreadsheetId === testSpreadsheetId) {
    throw new Error('Live and test spreadsheet IDs must be different.');
  }
  const deployment = await loadDeployment();
  if (!deployment?.deploymentId) {
    throw new Error('Deploy and bootstrap the authorized gateway before promotion.');
  }
  console.log(`Gateway version: ${deployment.versionNumber}`);
  console.log(`Target live workbook: ${spreadsheetId}`);
  if (!args.commit) {
    console.log('Dry run only. Add --commit to point the authorized gateway at the live workbook.');
    return;
  }
  const { client } = await gatewayAuth();
  const script = scriptService(client);
  const configured = await runGateway(script, deployment.deploymentId, 'setConfiguredSpreadsheet', [{ spreadsheetId }]);
  const health = await runGateway(script, deployment.deploymentId, 'healthCheck');
  if (!configured?.configured || !health?.ok || configured.workbook !== health.workbook) {
    throw new Error('Live gateway verification failed.');
  }
  console.log(`Gateway promoted and verified for live workbook; timezone ${health.timezone}.`);
});