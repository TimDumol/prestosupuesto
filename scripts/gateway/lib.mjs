import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { google } from 'googleapis';
import { authorize, SHEETS_SCOPE } from '../sheets/lib.mjs';

export const SCRIPT_PROJECTS_SCOPE = 'https://www.googleapis.com/auth/script.projects';
export const SCRIPT_DEPLOYMENTS_SCOPE = 'https://www.googleapis.com/auth/script.deployments';
export const USERINFO_EMAIL_SCOPE = 'https://www.googleapis.com/auth/userinfo.email';
export const GATEWAY_SCOPES = [
  SCRIPT_PROJECTS_SCOPE,
  SCRIPT_DEPLOYMENTS_SCOPE,
  SHEETS_SCOPE,
  USERINFO_EMAIL_SCOPE,
];

export async function gatewayAuth() {
  const tokenPath = path.resolve(
    process.cwd(),
    process.env.GOOGLE_APPS_SCRIPT_TOKEN ?? '.google/apps-script-deployment-token.json',
  );
  return authorize({ scopes: GATEWAY_SCOPES, tokenPath });
}

export function deploymentPath() {
  return path.resolve(
    process.cwd(),
    process.env.GOOGLE_APPS_SCRIPT_DEPLOYMENT ?? '.google/apps-script-deployment.json',
  );
}

export async function loadDeployment() {
  try {
    return JSON.parse(await readFile(deploymentPath(), 'utf8'));
  } catch (error) {
    if (error?.code === 'ENOENT') return null;
    throw error;
  }
}

export function scriptService(auth) {
  return google.script({ version: 'v1', auth });
}

export function executionError(operation) {
  const details = operation?.error?.details?.[0];
  const scriptError = details?.errorMessage;
  return scriptError ?? operation?.error?.message ?? 'Unknown Apps Script execution error.';
}

export async function runGateway(script, deploymentId, functionName, parameters = []) {
  const response = await script.scripts.run({
    scriptId: deploymentId,
    requestBody: { function: functionName, parameters },
  });
  if (response.data.error) throw new Error(executionError(response.data));
  return response.data.response?.result;
}
