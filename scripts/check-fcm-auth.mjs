#!/usr/bin/env node
// Validate server authorization without delivering a notification to any device.
import { readFile } from 'node:fs/promises';
import { google } from 'googleapis';

try {
  const path = process.argv[2] || process.env.FCM_SERVICE_ACCOUNT_FILE;
  if (!path) throw new Error('Pass the path to the FCM service-account JSON.');
  const credentials = JSON.parse(await readFile(path, 'utf8'));
  if (credentials.type !== 'service_account' || !/^[a-zA-Z0-9-]+$/.test(credentials.project_id))
    throw new Error('Invalid service-account configuration.');
  const auth = new google.auth.GoogleAuth({ credentials, scopes: ['https://www.googleapis.com/auth/firebase.messaging'] });
  const token = await auth.getAccessToken();
  const response = await fetch(`https://fcm.googleapis.com/v1/projects/${credentials.project_id}/messages:send`, {
    method: 'POST', headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ validate_only: true, message: {
      token: 'presto-validation-placeholder-not-a-real-device', data: { test: 'authorization-only' },
    } }),
  });
  const body = await response.json();
  if (response.status === 400 && body.error?.status === 'INVALID_ARGUMENT') {
    console.log('FCM authentication and API access succeeded. The placeholder device token was rejected as expected; no notification was sent.');
  } else if (response.ok) {
    console.log('FCM dry-run validation succeeded; no notification was sent.');
  } else {
    console.error(`FCM authorization check failed: HTTP ${response.status}, ${body.error?.status || 'unknown error'}. Check API enablement and the service account IAM role.`);
    process.exitCode = 1;
  }
} catch {
  // OAuth library errors may contain private request material; never print them.
  console.error('FCM authorization check failed. Check the credential file, network access and API configuration.');
  process.exitCode = 1;
}
