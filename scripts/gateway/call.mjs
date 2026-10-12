import { readFile } from 'node:fs/promises';
import path from 'node:path';
import {
  finish,
  loadRecord,
  option,
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
  const deployment = await loadDeployment();
  if (!deployment?.deploymentId) throw new Error('Deploy the test gateway first.');
  const functionName = String(option(args, 'function', 'healthCheck'));
  let parameter;
  if (option(args, 'data') || args.set) {
    parameter = await loadRecord(args);
  } else if (option(args, 'json')) {
    parameter = JSON.parse(await readFile(path.resolve(process.cwd(), String(option(args, 'json'))), 'utf8'));
  }
  const { client } = await gatewayAuth();
  const result = await runGateway(
    scriptService(client),
    deployment.deploymentId,
    functionName,
    parameter === undefined ? [] : [parameter],
  );
  console.log(JSON.stringify(result, null, 2));
});
