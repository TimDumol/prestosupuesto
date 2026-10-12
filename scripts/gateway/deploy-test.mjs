import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import {
  extractSpreadsheetId,
  finish,
  option,
  parseArgs,
} from '../sheets/lib.mjs';
import {
  deploymentPath,
  gatewayAuth,
  loadDeployment,
  runGateway,
  scriptService,
} from './lib.mjs';

finish(async () => {
  const args = parseArgs(process.argv.slice(2));
  const testSpreadsheetId = extractSpreadsheetId(process.env.GOOGLE_SHEETS_TEST_SPREADSHEET_ID);
  const productionId = process.env.GOOGLE_SHEETS_SPREADSHEET_ID
    ? extractSpreadsheetId(process.env.GOOGLE_SHEETS_SPREADSHEET_ID)
    : null;
  if (productionId && productionId === testSpreadsheetId) {
    throw new Error('Test and production spreadsheet IDs must be different.');
  }
  const createdBy = String(option(
    args,
    'created-by',
    process.env.GOOGLE_SHEETS_CREATED_BY ?? 'td_prestosupuesto',
  ));
  const source = await readFile(path.resolve(process.cwd(), 'apps-script/Code.js'), 'utf8');
  const manifestSource = await readFile(path.resolve(process.cwd(), 'apps-script/appsscript.json'), 'utf8');
  JSON.parse(manifestSource);
  const existing = await loadDeployment();
  console.log(`Test workbook: ${testSpreadsheetId}`);
  console.log(`Bound script: ${existing?.scriptId ? 'will update existing project' : 'will create a new project'}`);
  console.log(`API deployment: ${existing?.deploymentId ? 'will replace with a new immutable version' : 'will create'}`);
  if (!args.commit) {
    console.log('Dry run only. Add --commit to create/update and deploy the test gateway.');
    return;
  }

  const { client } = await gatewayAuth();
  const script = scriptService(client);
  let scriptId = existing?.scriptId;
  if (!scriptId) {
    const created = await script.projects.create({
      requestBody: {
        title: 'Presto Presupuesto Test Gateway',
        parentId: testSpreadsheetId,
      },
    });
    scriptId = created.data.scriptId;
    const output = deploymentPath();
    await mkdir(path.dirname(output), { recursive: true });
    await writeFile(output, JSON.stringify({
      scriptId,
      testSpreadsheetId,
      createdAt: new Date().toISOString(),
      configured: false,
    }, null, 2), 'utf8');
    console.log(`Created bound script project and saved its ID to ${output}.`);
  }
  await script.projects.updateContent({
    scriptId,
    requestBody: {
      files: [
        { name: 'appsscript', type: 'JSON', source: manifestSource },
        { name: 'Code', type: 'SERVER_JS', source },
      ],
    },
  });
  const version = await script.projects.versions.create({
    scriptId,
    requestBody: { description: `Presto Presupuesto test gateway ${new Date().toISOString()}` },
  });
  const deployment = await script.projects.deployments.create({
    scriptId,
    requestBody: {
      versionNumber: version.data.versionNumber,
      manifestFileName: 'appsscript',
      description: 'Presto Presupuesto test API executable',
    },
  });
  const deploymentId = deployment.data.deploymentId;
  const record = {
    scriptId,
    deploymentId,
    versionNumber: version.data.versionNumber,
    testSpreadsheetId,
    deployedAt: new Date().toISOString(),
  };
  const output = deploymentPath();
  await mkdir(path.dirname(output), { recursive: true });
  await writeFile(output, JSON.stringify(record, null, 2), 'utf8');
  console.log(`Gateway source deployed as version ${record.versionNumber}.`);

  if (!existing?.configured) {
    const configured = await runGateway(script, deploymentId, 'bootstrapTestConfiguration', [{
      spreadsheetId: testSpreadsheetId,
      createdBy,
    }]);
    record.configured = true;
    record.ownerEmail = configured.allowedUser;
    await writeFile(output, JSON.stringify(record, null, 2), 'utf8');
    console.log(`Gateway configured for its signed-in owner as ${createdBy}.`);
  }
  const health = await runGateway(script, deploymentId, 'healthCheck');
  console.log(`Gateway health: ${health.ok ? 'passed' : 'failed'}; timezone ${health.timezone}.`);
  console.log(`Private deployment metadata saved to ${output}.`);
});
