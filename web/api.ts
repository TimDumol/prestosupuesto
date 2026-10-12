import { mapBudgetCoverage } from "./lib/budgetLedger.ts";
import type { RawBudgetCoverage, IncomeTreatment } from "./lib/budgetLedger";
import type {
  FinanceSnapshot,
  ReconcileAccountsPayload,
  ServerSession,
  SubmitTransaction,
  GatewayTransaction,
  Currency,
  ReconciliationPreview,
  ReconciliationBatch,
  AccountHistoryDate,
} from './contracts';

type DbAccount = {
  id: string;
  name: string;
  accountType: string | null;
  currency: Currency | null;
  balanceNative: string | null;
  actualBalanceNative: string | null;
  reconciliationNative: string | null;
  balanceEur: string | null;
  actualBalanceEur: string | null;
  ledgerRevision: number;
};

type DbAccountHistoryDate = {
  date: string;
  totalEur: string | null;
  accounts: { accountId: string; name: string; currency: Currency; balanceNative: string; balanceEur: string | null }[];
};

type DbCategory = {
  categoryId: string;
  name: string;
  monthlyEur: string | null;
  expenseEur: string | null;
  remainingEur: string | null;
  reallocationImpactEur: string | null;
  carryoverEur: string | null;
  availableEur: string | null;
  adjustmentEur: string | null;
  netSpendingEur: string | null;
};

type DbTransaction = {
  createdBy?: string | null;
  createdAt?: string | null;
  correctsTransactionId: string | null;
  incomeTreatment: IncomeTreatment | null;
  expectedIncomeMonth: string | null;
  id: string;
  kind: string;
  effectiveDate: string;
  occurredAt?: string | null;
  description: string;
  amount: string;
  inputCurrency: Currency | null;
  toAmount: string | null;
  toCurrency: Currency | null;
  fromCategory: string | null;
  toCategory: string | null;
  fromCategoryId: string | null;
  toCategoryId: string | null;
  fromAccount: string | null;
  toAccount: string | null;
  fromAccountId: string | null;
  toAccountId: string | null;
  hasCategoryCorrection: boolean;
  revision: number;
  canEdit: boolean;
};

type DbReconciliationBatch = Omit<ReconciliationBatch, 'accounts'> & {
  accounts: (Omit<ReconciliationBatch['accounts'][number], 'observedBalanceNative' | 'closingBalanceNative' | 'varianceNative' | 'postMonthActivityNative'> & {
    observedBalanceNative: string | null;
    closingBalanceNative: string | null;
    varianceNative: string | null;
    postMonthActivityNative: string | null;
  })[];
};

type DbFinanceSnapshot = {
  budgetCoverage?: RawBudgetCoverage | null;
  asOf: string;
  budgetMonth: string;
  canReallocate: boolean;
  scheduledAllocations: { effectiveStartMonth: string; name: string; amountEur: string | null }[];
  transactionCount: number;
  recordedIncomeEur: string;
  incomeExpectation: { amount: string; currency: Currency; amountEur: string; recordedAmount: string; outstandingEur: string; arrivalDay: number };
  accountValuation: { asOf: string; totalEur: string; rates: { currency: Currency; date: string | null; unitsPerEur: string }[] };
  recentTransactions: DbTransaction[];
  budgets: DbCategory[];
  budgetHistory: { month: string; categories: DbCategory[]; status: "historical" | "open" | "closed"; carryoverProvisional: boolean }[];
  accounts: DbAccount[];
  accountHistory: DbAccountHistoryDate[];
  reconciliationStatus: { closingDate: string; batch: DbReconciliationBatch | null };
};


let latestSnapshot: DbFinanceSnapshot | null = null;
let snapshotSequence = 0;
let authGeneration = 0;

function clearAuthCache() {
  authGeneration += 1;
  snapshotSequence += 1;
  latestSnapshot = null;
  pendingReconciliation = null;
  pendingCloses.clear();
  try {
    sessionStorage.removeItem('presto-pending-reconciliation');
    for (let index = sessionStorage.length - 1; index >= 0; index -= 1) {
      const key = sessionStorage.key(index);
      if (key?.startsWith('presto-close-')) sessionStorage.removeItem(key);
    }
  } catch { /* Storage can be unavailable. */ }
}

export function invalidateServerSession() {
  clearAuthCache();
  if (typeof window !== 'undefined') window.dispatchEvent(new Event('presto-session-expired'));
}

export class FinanceApiError extends Error {
  status: number;
  constructor(status: number, message: string) { super(message); this.status = status; }
}

function mapReconciliationBatch(source: DbReconciliationBatch | null): ReconciliationBatch | null {
  if (!source) return null;
  const amount = (value: string | null) => value === null ? null : Number(value);
  return {
    ...source,
    accounts: source.accounts.map(account => ({
      ...account,
      observedBalanceNative: amount(account.observedBalanceNative),
      closingBalanceNative: amount(account.closingBalanceNative),
      varianceNative: amount(account.varianceNative),
      postMonthActivityNative: amount(account.postMonthActivityNative),
    })),
  };
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const generation = authGeneration;
  const headers = new Headers(init.headers);
  if (init.body) headers.set('Content-Type', 'application/json');
  const response = await fetch(path, {
    ...init,
    credentials: 'same-origin',
    headers,
  });
  const body = await response.json().catch(() => null) as { error?: string } | null;
  if (generation !== authGeneration) throw new FinanceApiError(401, 'Your session ended. Sign in again.');
  if (!response.ok) {
    if (response.status === 401) invalidateServerSession();
    throw new FinanceApiError(response.status, body?.error || `Request failed (${response.status}).`);
  }
  if (body === null) throw new Error('The finance server returned an invalid response. Refresh and retry.');
  return body as T;
}

function uuidV7() {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  let timestamp = Date.now();
  for (let index = 5; index >= 0; index -= 1) {
    bytes[index] = timestamp & 255;
    timestamp = Math.floor(timestamp / 256);
  }
  bytes[6] = (bytes[6] & 15) | 112;
  bytes[8] = (bytes[8] & 63) | 128;
  const hex = Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
  return hex.slice(0, 8) + '-' + hex.slice(8, 12) + '-' + hex.slice(12, 16) + '-' + hex.slice(16, 20) + '-' + hex.slice(20);
}

export function getServerSession() {
  return request<ServerSession>('/api/session');
}

export function startGoogleLogin() {
  window.location.assign('/auth/google');
}

export function startPocketIdLogin() {
  window.location.assign('/auth/pocket-id');
}

export async function signOutFromServer() {
  await request<{ signedOut: boolean }>('/api/logout', { method: 'POST' });
  clearAuthCache();
}

export async function getFinanceSnapshot(asOf: string, recentCount = 200): Promise<FinanceSnapshot> {
  const sequence = ++snapshotSequence;
  const query = new URLSearchParams({ asOf, recentCount: String(recentCount) });
  const source = await request<DbFinanceSnapshot>('/api/db/finance?' + query.toString());
  const amount = (value: string | null) => value === null ? null : Number(value);
  const category = (item: DbCategory) => ({
    name: item.name, monthlyEur: amount(item.monthlyEur), expenseEur: amount(item.expenseEur),
    carryoverEur: amount(item.carryoverEur), availableEur: amount(item.availableEur), adjustmentEur: amount(item.adjustmentEur), netSpendingEur: amount(item.netSpendingEur),
    remainingEur: amount(item.remainingEur), reallocationImpactEur: amount(item.reallocationImpactEur) ?? 0,
  });
  const mapped: FinanceSnapshot = {
    budgetCoverage: mapBudgetCoverage(source.budgetCoverage),
    generatedAt: new Date().toISOString(), asOf: source.asOf, budgetMonth: source.budgetMonth,
    canReallocate: source.canReallocate,
    scheduledAllocations: source.scheduledAllocations.map(item => ({ ...item, amountEur: amount(item.amountEur) })),
    transactionCount: source.transactionCount, budgets: source.budgets.map(category),
    recordedIncomeEur: Number(source.recordedIncomeEur),
    incomeExpectation: { ...source.incomeExpectation, amount: Number(source.incomeExpectation.amount), amountEur: Number(source.incomeExpectation.amountEur), recordedAmount: Number(source.incomeExpectation.recordedAmount), outstandingEur: Number(source.incomeExpectation.outstandingEur) },
    accountValuation: { ...source.accountValuation, totalEur: Number(source.accountValuation.totalEur), rates: source.accountValuation.rates.map(rate => ({ ...rate, unitsPerEur: Number(rate.unitsPerEur) })) },
    budgetHistory: source.budgetHistory.map(month => ({ month: month.month, status: month.status, carryoverProvisional: month.carryoverProvisional, categories: month.categories.map(category) })),
    accounts: source.accounts.map(item => ({
      name: item.name, type: item.accountType, currency: item.currency ?? 'EUR',
      balanceNative: amount(item.balanceNative), actualBalanceNative: amount(item.actualBalanceNative),
      reconciliationNative: amount(item.reconciliationNative), balanceEur: amount(item.balanceEur),
      actualBalanceEur: amount(item.actualBalanceEur),
    })),
    accountHistory: source.accountHistory.map((item): AccountHistoryDate => ({
      date: item.date, totalEur: amount(item.totalEur),
      accounts: item.accounts.map(account => ({
        ...account, balanceNative: Number(account.balanceNative), balanceEur: amount(account.balanceEur),
      })),
    })),
    reconciliationStatus: {
      closingDate: source.reconciliationStatus.closingDate,
      batch: mapReconciliationBatch(source.reconciliationStatus.batch),
    },
    recentTransactions: source.recentTransactions.map(mapTransaction),
  };
  if (sequence === snapshotSequence) latestSnapshot = source;
  return mapped;
}

function mapTransaction(item: DbTransaction): GatewayTransaction {
  const amount = (value: string | null) => value === null ? null : Number(value);
  return {
      row: 0, correctsTransactionId: item.correctsTransactionId, incomeTreatment: item.incomeTreatment, expectedIncomeMonth: item.expectedIncomeMonth, date: item.effectiveDate, occurredAt: item.occurredAt, description: item.description, amount: amount(item.amount),
      currency: item.inputCurrency, toAmount: amount(item.toAmount), fromCategory: item.fromCategory,
      toCategory: item.toCategory, fromAccount: item.fromAccount, toAccount: item.toAccount,
      transactionId: item.id, createdAt: item.createdAt ?? null, createdBy: item.createdBy ?? null, revision: String(item.revision),
      kind: item.kind as GatewayTransaction['kind'], canEdit: item.canEdit,
    };
}

export type LedgerQuery = { month?: string; search?: string; kind?: string; category?: string; account?: string; page?: number; pageSize?: number; sort?: string; direction?: 'asc' | 'desc' };
export async function getTransactions(query: LedgerQuery, signal?: AbortSignal) {
  const snapshot = latestSnapshot;
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) if (value !== undefined && value !== '') params.set(key, String(value));
  const result = await request<{ transactions: DbTransaction[]; total: number; page: number; pageSize: number }>('/api/db/transactions?' + params, { signal });
  // Editing a paginated historical row still uses the snapshot's account revisions.
  if (!signal?.aborted && snapshot && latestSnapshot === snapshot) {
    const rows = new Map(snapshot.recentTransactions.map(item => [item.id, item]));
    for (const item of result.transactions) {
      if ((rows.get(item.id)?.revision ?? 0) <= item.revision) rows.set(item.id, item);
    }
    latestSnapshot.recentTransactions = [...rows.values()];
  }
  return { ...result, transactions: result.transactions.map(mapTransaction) };
}

export function submitTransaction(payload: SubmitTransaction) {
  return request<{ duplicate: boolean; transaction: GatewayTransaction }>('/api/db/transactions', {
    method: 'POST', body: JSON.stringify({ operationId: payload.transactionId, ...transactionWritePayload(payload) }),
  });
}

function transactionWritePayload(payload: SubmitTransaction) {
  if (!latestSnapshot) throw new Error('Refresh finance data before posting.');
  const categoryId = (name: string | undefined) => {
    if (!name) return undefined;
    const category = latestSnapshot!.budgets.find(item => item.name === name);
    if (!category) throw new Error('Refresh finance data to resolve the selected category.');
    return category.categoryId;
  };
  const accountId = (name: string | undefined) => {
    if (!name || payload.kind === 'reallocate') return undefined;
    const account = latestSnapshot!.accounts.find(item => item.name === name);
    if (!account) throw new Error('Refresh finance data to resolve the selected account.');
    return account.id;
  };
  return {
    transactionId: payload.transactionId,
    incomeTreatment: payload.incomeTreatment,
    correctsTransactionId: payload.correctsTransactionId,
    expectedIncomeMonth: payload.expectedIncomeMonth,
    kind: payload.kind,
    date: payload.date,
    occurredAt: payload.occurredAt,
    dateOnly: payload.dateOnly,
    description: payload.description,
    amount: String(payload.amount),
    currency: payload.kind === 'reallocate' ? undefined : payload.currency,
    toAmount: payload.kind === 'reallocate' || payload.toAmount === undefined ? undefined : String(payload.toAmount),
    fromCategoryId: categoryId(payload.fromCategory),
    toCategoryId: categoryId(payload.toCategory),
    fromAccountId: accountId(payload.fromAccount),
    toAccountId: accountId(payload.toAccount),
  };
}

export function editTransaction(payload: SubmitTransaction, expectedRevision: number) {
  if (!latestSnapshot) throw new Error('Refresh finance data before editing.');
  const previous = latestSnapshot.recentTransactions.find(item => item.id === payload.transactionId);
  if (!previous) throw new Error('Refresh finance data before editing this transaction.');
  const write = transactionWritePayload(payload);
  const involvedIds = new Set([previous.fromAccountId, previous.toAccountId, write.fromAccountId, write.toAccountId].filter((id): id is string => Boolean(id)));
  const expectedAccountRevisions = latestSnapshot.accounts
    .filter(account => involvedIds.has(account.id))
    .map(account => ({ accountId: account.id, revision: account.ledgerRevision }));
  return request<{ updated: boolean; transactionId: string; revision: number }>('/api/db/transactions/edit', {
    method: 'POST', body: JSON.stringify({
      operationId: uuidV7(), ...write, expectedRevision, expectedAccountRevisions,
    }),
  });
}

export function deleteTransaction(transactionId: string, operationId = uuidV7()) {
  if (!latestSnapshot) throw new Error('Refresh finance data before deleting.');
  const previous = latestSnapshot.recentTransactions.find(item => item.id === transactionId);
  if (!previous) throw new Error('Refresh finance data before deleting this transaction.');
  const involvedIds = new Set([previous.fromAccountId, previous.toAccountId].filter((id): id is string => Boolean(id)));
  const expectedAccountRevisions = latestSnapshot.accounts
    .filter(account => involvedIds.has(account.id))
    .map(account => ({ accountId: account.id, revision: account.ledgerRevision }));
  return request<{ deleted: boolean; transactionId: string }>('/api/db/transactions/delete', {
    method: 'POST', body: JSON.stringify({
      operationId, transactionId, expectedRevision: Number(previous.revision),
      expectedAccountRevisions,
    }),
  });
}

type DbReconciliationPreview = Omit<ReconciliationPreview, 'totalEur' | 'closingTotalEur' | 'accounts' | 'rates' | 'reconciliation'> & {
  totalEur: string; closingTotalEur: string;
  reconciliation: DbReconciliationBatch | null;
  accounts: (Omit<ReconciliationPreview['accounts'][number], 'balanceNative' | 'closingBalanceNative' | 'activityNative' | 'existingAdjustmentNative' | 'transactions'> & {
    balanceNative: string; closingBalanceNative: string; activityNative: string; existingAdjustmentNative: string;
    transactions: (Omit<ReconciliationPreview['accounts'][number]['transactions'][number], 'netNative'> & { netNative: string })[];
  })[];
  rates: (Omit<ReconciliationPreview['rates'][number], 'unitsPerEur' | 'closingUnitsPerEur'> & { unitsPerEur: string; closingUnitsPerEur: string })[];
};

export async function getReconciliationPreview(observedDate: string, closingMonth?: string): Promise<ReconciliationPreview> {
  const source = await request<DbReconciliationPreview>('/api/db/reconciliation/preview?' + new URLSearchParams({ observedDate, ...(closingMonth ? { closingMonth } : {}) }));
  return { ...source, totalEur: Number(source.totalEur), closingTotalEur: Number(source.closingTotalEur),
    reconciliation: mapReconciliationBatch(source.reconciliation),
    rates: source.rates.map(rate => ({ ...rate, unitsPerEur: Number(rate.unitsPerEur), closingUnitsPerEur: Number(rate.closingUnitsPerEur) })),
    accounts: source.accounts.map(account => ({ ...account, balanceNative: Number(account.balanceNative), closingBalanceNative: Number(account.closingBalanceNative), activityNative: Number(account.activityNative), existingAdjustmentNative: Number(account.existingAdjustmentNative), transactions: account.transactions.map(item => ({ ...item, netNative: Number(item.netNative) })) })),
  };
}

let pendingReconciliation: { fingerprint: string; body: string } | null = null;

export async function reconcileAccounts(payload: ReconcileAccountsPayload, preview: ReconciliationPreview) {
  if (payload.date !== preview.closingDate) throw new Error('Refresh reconciliation before posting.');
  const fingerprint = JSON.stringify({ date: payload.date, observedAt: preview.observedAt, observedDate: preview.observedDate,
    revisions: preview.accounts.map(account => [account.id, account.ledgerRevision]),
    accounts: payload.accounts.map(({name, actualBalance}) => ({name, actualBalance})) });
  let cached = pendingReconciliation;
  try {
    const stored = JSON.parse(sessionStorage.getItem('presto-pending-reconciliation') ?? 'null');
    if (cached?.fingerprint !== fingerprint && stored?.fingerprint === fingerprint && typeof stored.body === 'string') {
      const requestBody = JSON.parse(stored.body);
      if (requestBody && typeof requestBody.operationId === 'string' && Array.isArray(requestBody.accounts)) cached = stored;
    }
  } catch { /* Retain retry identity in memory when storage is unavailable or corrupt. */ }
  const body = cached?.fingerprint === fingerprint ? cached.body : JSON.stringify({
    operationId: payload.accounts[0]?.transactionId, date: preview.closingDate,
    observedAt: new Date().toISOString(), observedDate: preview.observedDate,
    accounts: payload.accounts.map(item => {
      const account = preview.accounts.find(account => account.name === item.name);
      if (!account) throw new Error('Refresh reconciliation to resolve the selected account.');
      return { accountId: account.id, actualBalance: String(item.actualBalance), transactionId: item.transactionId, expectedRevision: account.ledgerRevision };
    }),
  });
  pendingReconciliation = { fingerprint, body };
  try { sessionStorage.setItem('presto-pending-reconciliation', JSON.stringify(pendingReconciliation)); } catch { /* In-memory retries remain available. */ }
  const result = await request<{ recorded: boolean }>('/api/db/reconciliation', { method: 'POST', body });
  pendingReconciliation = null;
  try { sessionStorage.removeItem('presto-pending-reconciliation'); } catch { /* Reconciliation has already succeeded. */ }
  return result;
}

export function saveExpectedIncome(effectiveStartMonth: string, amount: number, currency: Currency, arrivalDay: number) {
  return request<{ saved: boolean }>('/api/db/finance/expected-income', {
    method: 'POST', body: JSON.stringify({ effectiveStartMonth, amount: String(amount), currency, arrivalDay }),
  });
}

export function saveMonthlyBudget(effectiveStartMonth: string, categories: { name: string; amount: string }[]) {
  return request<{ saved: boolean }>('/api/db/finance/monthly-budget', {
    method: 'POST', body: JSON.stringify({ effectiveStartMonth, categories }),
  });
}

const pendingCloses = new Map<string, string>();

export async function closeBudgetMonth(month: string) {
  const key = 'presto-close-' + month;
  let operationId = pendingCloses.get(month);
  if (!operationId) {
    try { operationId = sessionStorage.getItem(key) ?? undefined; } catch { /* Retain retries in memory. */ }
  }
  operationId ??= uuidV7();
  pendingCloses.set(month, operationId);
  try { sessionStorage.setItem(key, operationId); } catch { /* Retain retries in memory. */ }
  const result = await request<{ closed: boolean }>('/api/budget/close', {
    method: 'POST', body: JSON.stringify({ month, operationId }),
  });
  pendingCloses.delete(month);
  try { sessionStorage.removeItem(key); } catch { /* The month has already closed. */ }
  return result;
}

export function completeExpectedIncome(expectedMonth: string, operationId = uuidV7()) {
  return request("/api/budget/income-complete", { method: "POST", body: JSON.stringify({ expectedMonth, operationId }) });
}

export function cancelMonthlyBudget(month: string, name: string) {
  return request('/api/db/finance/monthly-budget/cancel', { method: 'POST', body: JSON.stringify({ effectiveStartMonth: month, categories: [{ name, amount: '0' }] }) });
}
type HistoryTransaction = { description?: string; amount?: string; inputCurrency?: string; fromAccountName?: string; toAccountName?: string; fromCategoryName?: string; toCategoryName?: string; date?: string; fromCategoryId?: string; toCategoryId?: string };
export type TransactionHistoryEntry = { revision: number; at: string; by: string | null; details: { request?: HistoryTransaction; transaction?: HistoryTransaction } };
export function getTransactionHistory(id: string) {
  return request<{ history: TransactionHistoryEntry[] }>('/api/db/transactions/history?' + new URLSearchParams({ id }));
}

export function createBudgetCategory(name: string, monthlyAmount: string) {
  return request('/api/db/finance/categories', {
    method: 'POST', body: JSON.stringify({ name, monthlyAmount }),
  });
}
