import type { BudgetCoverage, IncomeTreatment } from "./lib/budgetLedger";
export type Currency = 'EUR' | 'PHP' | 'USD';
export type TransactionKind = 'expense' | 'income' | 'transfer' | 'reallocate';

export type GatewayBudget = {
  name: string;
  monthlyEur: number | null;
  expenseEur: number | null;
  remainingEur: number | null;
  carryoverEur?: number | null;
  availableEur?: number | null;
  adjustmentEur?: number | null;
  netSpendingEur?: number | null;
};

export type GatewayBudgetHistoryCategory = {
  name: string;
  monthlyEur: number | null;
  expenseEur: number | null;
  remainingEur: number | null;
  carryoverEur?: number | null;
  availableEur?: number | null;
  adjustmentEur?: number | null;
  netSpendingEur?: number | null;
  reallocationImpactEur: number;
};

export type GatewayBudgetHistoryMonth = {
  month: string;
  categories: GatewayBudgetHistoryCategory[];
  status?: "historical" | "open" | "closed";
  carryoverProvisional?: boolean;
};

export type GatewayAccount = {
  name: string;
  type: string | null;
  currency: Currency;
  balanceNative: number | null;
  actualBalanceNative: number | null;
  reconciliationNative: number | null;
  balanceEur: number | null;
  actualBalanceEur: number | null;
};

export type AccountHistoryBalance = {
  accountId: string;
  name: string;
  currency: Currency;
  balanceNative: number;
  balanceEur: number | null;
};

export type AccountHistoryDate = {
  date: string;
  totalEur: number | null;
  accounts: AccountHistoryBalance[];
};

export type ReconciliationAccountInput = {
  name: string;
  actualBalance: number;
  transactionId: string;
};

export type ReconcileAccountsPayload = {
  date: string;
  accounts: ReconciliationAccountInput[];
};

export type GatewayTransaction = {
  correctsTransactionId?: string | null;
  incomeTreatment?: IncomeTreatment | null;
  expectedIncomeMonth?: string | null;
  row: number;
  date: string | null;
  occurredAt?: string | null;
  description: string;
  amount: number | null;
  currency: Currency | null;
  toAmount: number | null;
  fromCategory: string | null;
  toCategory: string | null;
  fromAccount: string | null;
  toAccount: string | null;
  transactionId: string | null;
  kind?: TransactionKind | 'reconciliation';
  canEdit?: boolean;
  createdAt: string | null;
  createdBy: string | null;
  revision: string;
};

export type FinanceSnapshot = {
  budgetCoverage?: BudgetCoverage | null;
  generatedAt: string;
  asOf: string;
  budgetMonth: string;
  canReallocate: boolean;
  scheduledAllocations?: { effectiveStartMonth: string; name: string; amountEur: number | null }[];
  transactionCount: number;
  recordedIncomeEur: number;
  incomeExpectation: { amount: number; currency: Currency; amountEur: number; recordedAmount: number; outstandingEur: number; arrivalDay: number };
  accountValuation: { asOf: string; totalEur: number; rates: { currency: Currency; date: string | null; unitsPerEur: number }[] };
  recentTransactions: GatewayTransaction[];
  budgets: GatewayBudget[];
  budgetHistory?: GatewayBudgetHistoryMonth[];
  accounts: GatewayAccount[];
  accountHistory: AccountHistoryDate[];
  reconciliationStatus: { closingDate: string; batch: ReconciliationBatch | null };
};

export type ReconciliationBatch = {
  effectiveDate: string;
  submittedAt: string;
  observedAt: string | null;
  observedDate: string | null;
  isComplete: boolean;
  isCurrent?: boolean;
  reconciledAccountCount: number;
  totalAccountCount: number;
  accounts: {
    id: string;
    name: string;
    accountType: string | null;
    currency: Currency | null;
    observedBalanceNative: number | null;
    closingBalanceNative: number | null;
    varianceNative: number | null;
    postMonthActivityNative: number | null;
  }[];
};

export type SubmitTransaction = {
  correctsTransactionId?: string;
  incomeTreatment?: Exclude<IncomeTreatment, "legacy">;
  expectedIncomeMonth?: string;
  occurredAt?: string | null;
  dateOnly?: boolean;
  transactionId: string;
  kind: TransactionKind;
  date: string;
  description: string;
  amount: number;
  currency?: Currency;
  toAmount?: number;
  fromCategory?: string;
  toCategory?: string;
  fromAccount?: string;
  toAccount?: string;
};

export type ServerSession = {
  authenticated: boolean;
  email: string | null;
  pocketAuthenticated: boolean;
  pocketUsername: string | null;
};

export type ReconciliationPreview = {
  closingDate: string;
  observedAt: string;
  observedDate: string;
  totalEur: number;
  closingTotalEur: number;
  reconciliation: ReconciliationBatch | null;
  rates: { currency: Currency; unitsPerEur: number; date: string | null; closingUnitsPerEur: number; closingRateDate: string | null }[];
  accounts: {
    id: string; name: string; type: string | null; currency: Currency;
    balanceNative: number; closingBalanceNative: number; activityNative: number; existingAdjustmentNative: number; ledgerRevision: number;
    transactions: { id: string; revision: number; date: string; occurredAt?: string | null; description: string; netNative: number }[];
  }[];
};
