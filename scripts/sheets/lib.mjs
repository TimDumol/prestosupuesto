import { authenticate } from '@google-cloud/local-auth';
import { google } from 'googleapis';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

export const SHEETS_SCOPE = 'https://www.googleapis.com/auth/spreadsheets';

export function parseArgs(argv) {
  const parsed = { _: [] };

  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (!token.startsWith('--')) {
      parsed._.push(token);
      continue;
    }

    const equalsAt = token.indexOf('=');
    const key = token.slice(2, equalsAt === -1 ? undefined : equalsAt);
    let value = equalsAt === -1 ? undefined : token.slice(equalsAt + 1);

    if (value === undefined && argv[index + 1] && !argv[index + 1].startsWith('--')) {
      value = argv[index + 1];
      index += 1;
    }

    value ??= true;
    if (Object.hasOwn(parsed, key)) {
      parsed[key] = Array.isArray(parsed[key]) ? [...parsed[key], value] : [parsed[key], value];
    } else {
      parsed[key] = value;
    }
  }

  return parsed;
}

export function option(args, name, fallback) {
  const value = args[name];
  return Array.isArray(value) ? value.at(-1) : value ?? fallback;
}

export function optionList(args, name) {
  const value = args[name];
  if (value === undefined) return [];
  return Array.isArray(value) ? value : [value];
}

export function extractSpreadsheetId(value) {
  if (!value) throw new Error('Set GOOGLE_SHEETS_SPREADSHEET_ID or pass --spreadsheet.');
  const match = String(value).match(/\/spreadsheets\/d\/([a-zA-Z0-9_-]+)/);
  return match?.[1] ?? String(value).trim();
}

export function quoteSheetTitle(title) {
  return `'${String(title).replaceAll("'", "''")}'`;
}

export function columnName(columnNumber) {
  if (!Number.isInteger(columnNumber) || columnNumber < 1) {
    throw new Error('Column number must be a positive integer.');
  }

  let value = columnNumber;
  let result = '';
  while (value > 0) {
    value -= 1;
    result = String.fromCharCode(65 + (value % 26)) + result;
    value = Math.floor(value / 26);
  }
  return result;
}

export function normalizeHeader(value) {
  return String(value).trim().toLocaleLowerCase().replace(/[^\p{L}\p{N}]+/gu, '');
}

export function resolveRecord(headers, record) {
  const headerLookup = new Map();
  headers.forEach((header, index) => {
    const normalized = normalizeHeader(header);
    if (!normalized) return;
    if (headerLookup.has(normalized)) {
      throw new Error(`Duplicate header after normalization: "${header}".`);
    }
    headerLookup.set(normalized, { header, index });
  });

  const resolved = new Map();
  const unknown = [];
  for (const [key, value] of Object.entries(record)) {
    const match = headerLookup.get(normalizeHeader(key));
    if (!match) {
      unknown.push(key);
      continue;
    }
    if (resolved.has(match.index)) {
      throw new Error(`More than one input field maps to the "${match.header}" column.`);
    }
    resolved.set(match.index, { header: match.header, value });
  }

  if (unknown.length) {
    throw new Error(`Input fields not found in the header row: ${unknown.join(', ')}`);
  }
  return resolved;
}

function parseAssignmentValue(value) {
  const trimmed = value.trim();
  if (trimmed === 'true') return true;
  if (trimmed === 'false') return false;
  if (trimmed === 'null') return null;
  if (trimmed !== '' && Number.isFinite(Number(trimmed))) return Number(trimmed);
  return value;
}

export function assignments(values) {
  const result = {};
  for (const assignment of values) {
    const equalsAt = String(assignment).indexOf('=');
    if (equalsAt < 1) throw new Error(`Expected Header=Value, received "${assignment}".`);
    const key = String(assignment).slice(0, equalsAt).trim();
    const value = String(assignment).slice(equalsAt + 1);
    result[key] = parseAssignmentValue(value);
  }
  return result;
}

export async function loadRecord(args, optionName = 'data', assignmentName = 'set') {
  let record = {};
  const dataFile = option(args, optionName);
  if (dataFile) {
    const contents = await readFile(path.resolve(process.cwd(), String(dataFile)), 'utf8');
    record = JSON.parse(contents);
    if (!record || Array.isArray(record) || typeof record !== 'object') {
      throw new Error(`${dataFile} must contain one JSON object.`);
    }
  }
  return { ...record, ...assignments(optionList(args, assignmentName)) };
}

async function loadOAuthToken(tokenPath) {
  try {
    return google.auth.fromJSON(JSON.parse(await readFile(tokenPath, 'utf8')));
  } catch (error) {
    if (error?.code === 'ENOENT') return null;
    throw error;
  }
}

async function saveOAuthToken(client, credentialsPath, tokenPath) {
  const credentials = JSON.parse(await readFile(credentialsPath, 'utf8'));
  const keys = credentials.installed ?? credentials.web;
  if (!keys?.client_id || !keys?.client_secret || !client.credentials.refresh_token) {
    throw new Error('OAuth credentials did not include the data needed to cache a refresh token.');
  }

  await mkdir(path.dirname(tokenPath), { recursive: true });
  await writeFile(tokenPath, JSON.stringify({
    type: 'authorized_user',
    client_id: keys.client_id,
    client_secret: keys.client_secret,
    refresh_token: client.credentials.refresh_token,
  }, null, 2), { mode: 0o600 });
}

export async function authorize(options = {}) {
  const scopes = options.scopes ?? [SHEETS_SCOPE];
  const serviceAccountPath = process.env.GOOGLE_APPLICATION_CREDENTIALS;
  if (serviceAccountPath) {
    const auth = new google.auth.GoogleAuth({
      keyFile: path.resolve(process.cwd(), serviceAccountPath),
      scopes,
    });
    return { client: await auth.getClient(), mode: 'service-account' };
  }

  const credentialsPath = path.resolve(
    process.cwd(),
    process.env.GOOGLE_OAUTH_CREDENTIALS ?? '.google/credentials.json',
  );
  const tokenPath = path.resolve(process.cwd(), options.tokenPath
    ?? process.env.GOOGLE_OAUTH_TOKEN
    ?? '.google/token.json');
  const savedClient = await loadOAuthToken(tokenPath);
  if (savedClient) return { client: savedClient, mode: 'oauth-token' };

  const client = await authenticate({ keyfilePath: credentialsPath, scopes });
  await saveOAuthToken(client, credentialsPath, tokenPath);
  return { client, mode: 'oauth-browser' };
}

export async function runtime(args) {
  const spreadsheetId = extractSpreadsheetId(
    option(args, 'spreadsheet', process.env.GOOGLE_SHEETS_SPREADSHEET_ID),
  );
  const { client, mode } = await authorize();
  return {
    spreadsheetId,
    mode,
    sheets: google.sheets({ version: 'v4', auth: client }),
  };
}

export function sheetOption(args, kind = 'transactions') {
  const envName = kind === 'budgets'
    ? 'GOOGLE_SHEETS_BUDGETS_SHEET'
    : 'GOOGLE_SHEETS_TRANSACTIONS_SHEET';
  const fallback = kind === 'budgets' ? 'Budgets' : 'Transactions';
  return String(option(args, 'sheet', process.env[envName] ?? fallback));
}

export async function headersFor(sheets, spreadsheetId, sheet, headerRow = 1) {
  if (!Number.isInteger(headerRow) || headerRow < 1) throw new Error('--header-row must be positive.');
  const range = `${quoteSheetTitle(sheet)}!${headerRow}:${headerRow}`;
  const response = await sheets.spreadsheets.values.get({ spreadsheetId, range });
  const headers = response.data.values?.[0] ?? [];
  if (!headers.length) throw new Error(`No headers found in ${range}.`);
  return headers.map((header) => String(header));
}

export function printResolved(resolved, before = []) {
  console.table([...resolved.entries()].map(([index, item]) => ({
    Column: columnName(index + 1),
    Header: item.header,
    ...(before.length ? { Before: before[index] ?? '' } : {}),
    After: item.value ?? '',
  })));
}

export function errorMessage(error) {
  return error?.response?.data?.error?.message ?? error?.message ?? String(error);
}

export function finish(main) {
  main().catch((error) => {
    console.error(`Google Sheets command failed: ${errorMessage(error)}`);
    process.exitCode = 1;
  });
}
