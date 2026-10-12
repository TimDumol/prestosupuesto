import { readFile } from 'node:fs/promises';
import { google } from 'googleapis';

try {
  const credentials = JSON.parse(await readFile('.google/credentials.json', 'utf8'));
  const number = credentials.installed.client_id.match(/^(\d+)-/)[1];
  const token = JSON.parse(await readFile('.google/cloud-admin-token.json', 'utf8'));
  const auth = google.auth.fromJSON(token);
  const usage = google.serviceusage({ version: 'v1', auth });
  const name = `projects/${number}/services/drive.googleapis.com`;
  const current = await usage.services.get({ name });
  if (current.data.state === 'ENABLED') {
    console.log('Google Drive API already enabled.');
  } else {
    let operation = (await usage.services.enable({ name })).data;
    for (let attempt = 0; !operation.done && attempt < 45; attempt++) {
      await new Promise(resolve => setTimeout(resolve, 1000));
      operation = (await usage.operations.get({ name: operation.name })).data;
    }
    if (!operation.done || operation.error) throw new Error(operation.error?.message ?? 'API enablement still pending.');
    console.log('Google Drive API enabled.');
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
