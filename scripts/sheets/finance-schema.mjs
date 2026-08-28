import { randomBytes } from 'node:crypto';

export const TRANSACTION_HEADERS = [
  'Date', 'Description', 'Amount', 'Currency', 'To Amount', 'From Category',
  'To Category', 'From', 'To', 'From Account Currency',
  'From Account Currency Exchange EUR', 'From Account Amount EUR',
  'From Account Currency Exchange Rate Native', 'From Account Amount Native',
  'To Account Currency', 'To Account Currency Exchange EUR', 'To Account Amount EUR',
];

export const TRANSACTION_FORMULA_COLUMNS = [3, 4, 9, 10, 11, 12, 13, 14, 15, 16];
export const TRANSACTION_INPUT_COLUMNS = [0, 1, 2, 5, 6, 7, 8];
export const TRANSACTION_METADATA_HEADERS = ['Transaction ID', 'Created At', 'Created By'];
export const TRANSACTION_FULL_HEADERS = [...TRANSACTION_HEADERS, ...TRANSACTION_METADATA_HEADERS];
export const SUPPORTED_CURRENCIES = ['EUR', 'PHP', 'USD'];

export function uuidV7(timestamp = Date.now(), random = randomBytes(16)) {
  if (!Number.isInteger(timestamp) || timestamp < 0 || timestamp > 0xffffffffffff) {
    throw new Error('UUIDv7 timestamp must be a non-negative 48-bit integer.');
  }
  if (!(random instanceof Uint8Array) || random.length < 16) {
    throw new Error('UUIDv7 random input must contain at least 16 bytes.');
  }
  const bytes = Uint8Array.from(random.slice(0, 16));
  let milliseconds = BigInt(timestamp);
  for (let index = 5; index >= 0; index -= 1) {
    bytes[index] = Number(milliseconds & 0xffn);
    milliseconds >>= 8n;
  }
  bytes[6] = (bytes[6] & 0x0f) | 0x70;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = [...bytes].map((value) => value.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function requireValue(value, name) {
  if (value === undefined || value === null || value === '') throw new Error(`${name} is required.`);
  return String(value);
}

export function sheetsSerial(dateText) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(dateText)) throw new Error('Date must use YYYY-MM-DD.');
  const timestamp = Date.parse(`${dateText}T00:00:00Z`);
  if (!Number.isFinite(timestamp)) throw new Error(`Invalid date: ${dateText}.`);
  return (timestamp - Date.UTC(1899, 11, 30)) / 86_400_000;
}

export function buildTransaction({
  kind,
  date,
  amount,
  description,
  fromCategory,
  toCategory,
  fromAccount,
  toAccount,
  toAmount,
  currency,
  testId = uuidV7(),
  descriptionPrefix = `[API TEST ${testId.slice(0, 8)}]`,
}) {
  const numericAmount = Number(amount);
  if (!Number.isFinite(numericAmount) || numericAmount <= 0) {
    throw new Error('Amount must be greater than zero.');
  }
  const values = new Map([
    [0, sheetsSerial(date)],
    [1, `${descriptionPrefix} ${String(description ?? `${kind} round-trip`)}`],
    [2, numericAmount],
  ]);
  if (currency !== undefined && currency !== null && currency !== '') {
    const normalizedCurrency = String(currency).toUpperCase();
    if (!SUPPORTED_CURRENCIES.includes(normalizedCurrency)) {
      throw new Error(`Currency must be one of: ${SUPPORTED_CURRENCIES.join(', ')}.`);
    }
    values.set(3, normalizedCurrency);
  }

  if (kind === 'expense') {
    values.set(5, requireValue(fromCategory, 'From category'));
    values.set(6, 'Expense');
    values.set(7, requireValue(fromAccount, 'From account'));
    values.set(8, 'Expense');
  } else if (kind === 'income') {
    values.set(5, 'Income');
    values.set(6, requireValue(toCategory, 'To category'));
    values.set(7, 'Income');
    values.set(8, requireValue(toAccount, 'To account'));
  } else if (kind === 'transfer') {
    values.set(5, 'Balance Transfer');
    values.set(6, 'Balance Transfer');
    values.set(7, requireValue(fromAccount, 'From account'));
    values.set(8, requireValue(toAccount, 'To account'));
  } else if (kind === 'reallocate') {
    values.set(5, requireValue(fromCategory, 'From category'));
    values.set(6, requireValue(toCategory, 'To category'));
    values.set(7, 'Reallocation');
    values.set(8, 'Reallocation');
  } else {
    throw new Error('Kind must be expense, income, transfer, or reallocate.');
  }

  let numericToAmount = null;
  if (toAmount !== undefined && toAmount !== null) {
    if (kind !== 'transfer') throw new Error('To amount is currently supported only for transfers.');
    numericToAmount = Number(toAmount);
    if (!Number.isFinite(numericToAmount) || numericToAmount <= 0) {
      throw new Error('To amount must be greater than zero.');
    }
    values.set(4, numericToAmount);
  }
  return { kind, date, amount: numericAmount, toAmount: numericToAmount, testId, values };
}

export function formulaTemplateProblems(formulaRow, rowNumber) {
  const exact = new Map([
    [4, `=C${rowNumber}`],
    [11, `=C${rowNumber}*K${rowNumber}`],
    [13, `=C${rowNumber}*M${rowNumber}`],
    [16, `=P${rowNumber}*E${rowNumber}`],
  ]);
  const snippets = new Map([
    [3, [`H${rowNumber}`, 'DGET(Accounts!']],
    [9, [`H${rowNumber}`, 'DGET(Accounts!']],
    [10, [`$A${rowNumber}`, "'Exchange Rates'!", `D${rowNumber}`]],
    [12, [`D${rowNumber}=J${rowNumber}`, "'Exchange Rates'!"]],
    [14, [`I${rowNumber}`, 'DGET(Accounts!', '"Expense"']],
    [15, [`$A${rowNumber}`, "'Exchange Rates'!", `O${rowNumber}`]],
  ]);
  const problems = [];
  for (const column of TRANSACTION_FORMULA_COLUMNS) {
    const formula = formulaRow[column];
    if (typeof formula !== 'string' || !formula.startsWith('=')) {
      problems.push(`${TRANSACTION_HEADERS[column]} is not a formula`);
      continue;
    }
    if (exact.has(column) && formula !== exact.get(column)) {
      problems.push(`${TRANSACTION_HEADERS[column]} formula differs from its contract`);
    }
    for (const snippet of snippets.get(column) ?? []) {
      if (!formula.includes(snippet)) {
        problems.push(`${TRANSACTION_HEADERS[column]} formula is missing ${snippet}`);
      }
    }
  }
  return problems;
}
