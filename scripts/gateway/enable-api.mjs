import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { google } from 'googleapis';
import { authorize, finish } from '../sheets/lib.mjs';

const CLOUD_PLATFORM_SCOPE = 'https://www.googleapis.com/auth/cloud-platform';

function projectNumber(credentials) {
  const clientId = credentials.installed?.client_id ?? credentials.web?.client_id;
  const match = String(clientId ?? '').match(/^(\d+)-/);
  if (!match) throw new Error('Could not derive the Google Cloud project number from the OAuth client ID.');
  return match[1];
}

function wait(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

finish(async () => {
  const credentialsPath = path.resolve(
    process.cwd(),
    process.env.GOOGLE_OAUTH_CREDENTIALS ?? '.google/credentials.json',
  );
  const credentials = JSON.parse(await readFile(credentialsPath, 'utf8'));
  const number = projectNumber(credentials);
  const tokenPath = path.resolve(
    process.cwd(),
    process.env.GOOGLE_CLOUD_ADMIN_TOKEN ?? '.google/cloud-admin-token.json',
  );
  const { client } = await authorize({ scopes: [CLOUD_PLATFORM_SCOPE], tokenPath });
  const serviceUsage = google.serviceusage({ version: 'v1', auth: client });
  const name = `projects/${number}/services/script.googleapis.com`;
  const current = await serviceUsage.services.get({ name });
  if (current.data.state === 'ENABLED') {
    console.log(`Apps Script API is already enabled for project ${number}.`);
    return;
  }
  const enabled = await serviceUsage.services.enable({ name });
  let operation = enabled.data;
  for (let attempt = 0; !operation.done && attempt < 60; attempt += 1) {
    await wait(1000);
    operation = (await serviceUsage.operations.get({ name: operation.name })).data;
  }
  if (!operation.done) throw new Error('Timed out waiting for Apps Script API enablement.');
  if (operation.error) throw new Error(operation.error.message ?? 'Apps Script API enablement failed.');
  console.log(`Apps Script API enabled for project ${number}.`);
});
