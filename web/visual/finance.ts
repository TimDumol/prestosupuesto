// Synthetic API-shaped household data. No exported personal records or credentials.
export const frozenTime = '2026-10-18T10:00:00+02:00';
const money = (value: number) => value.toFixed(2);
const categoryInputs = [
  ['Housing', 1200, 1200, 0], ['Groceries', 450, 492.35, 25],
  ['Transport', 160, 82.4, 0], ['Dining out', 180, 204.8, 0],
  ['Health & wellbeing', 120, 46.5, 80], ['Travel fund', 300, 0, 650],
  ['Subscriptions', 65, 39.98, 0], ['Household & utilities', 200, 143.75, 0],
] as const;
export const budgets = categoryInputs.map(([name, monthly, spent, carry], index) => ({
  categoryId: `category-${index}`, name, monthlyEur: money(monthly), expenseEur: money(spent),
  remainingEur: money(monthly + carry - spent), reallocationImpactEur: '0.00',
  carryoverEur: money(carry), availableEur: money(monthly + carry), adjustmentEur: '0.00', netSpendingEur: money(spent),
}));
export const accounts = [
  { id: 'checking', name: 'Household checking', accountType: 'Bank', currency: 'EUR', balanceNative: '2840.22', balanceEur: '2840.22' },
  { id: 'savings', name: 'Emergency savings', accountType: 'Savings', currency: 'EUR', balanceNative: '8650.00', balanceEur: '8650.00' },
  { id: 'usd', name: 'USD travel wallet', accountType: 'Bank', currency: 'USD', balanceNative: '550.00', balanceEur: '500.00' },
  { id: 'php', name: 'PHP family wallet', accountType: 'Bank', currency: 'PHP', balanceNative: '12500.00', balanceEur: '200.00' },
  { id: 'card', name: 'Household credit card', accountType: 'Credit card', currency: 'EUR', balanceNative: '-326.48', balanceEur: '-326.48' },
].map(account => ({ ...account, actualBalanceNative: account.balanceNative, actualBalanceEur: account.balanceEur, reconciliationNative: '0.00', ledgerRevision: 12 }));
export const transactions = [
  ['2026-10-17', 'Weekly groceries · neighbourhood market', '87.35', 'expense', 'Groceries', 'Household checking'],
  ['2026-10-16', 'Dinner with friends', '64.80', 'expense', 'Dining out', 'Household credit card'],
  ['2026-10-15', 'Monthly salary', '3200.00', 'income', 'Income', 'Income'],
  ['2026-10-12', 'Savings transfer', '300.00', 'transfer', 'Balance Transfer', 'Household checking'],
  ['2026-10-10', 'Family support', '2500.00', 'expense', 'Household & utilities', 'PHP family wallet'],
  ['2026-10-08', 'Streaming subscription', '14.99', 'expense', 'Subscriptions', 'Household credit card'],
  ['2026-10-06', 'Train tickets', '32.40', 'expense', 'Transport', 'Household checking'],
  ['2026-10-03', 'Pharmacy', '46.50', 'expense', 'Health & wellbeing', 'Household checking'],
  ['2026-10-01', 'October rent', '1200.00', 'expense', 'Housing', 'Household checking'],
].map(([effectiveDate, description, amount, kind, fromCategory, fromAccount], index) => ({
  id: `transaction-${index}`, effectiveDate, occurredAt: `${effectiveDate}T12:00:00+02:00`, description, amount, kind,
  inputCurrency: fromAccount === 'PHP family wallet' ? 'PHP' : 'EUR',
  toAmount: kind === 'income' || kind === 'transfer' ? amount : null, toCurrency: kind === 'income' || kind === 'transfer' ? 'EUR' : null,
  fromCategory, toCategory: kind === 'income' ? 'Income' : kind === 'transfer' ? 'Balance Transfer' : 'Expense',
  fromAccount, toAccount: kind === 'income' ? 'Household checking' : kind === 'transfer' ? 'Emergency savings' : 'Expense',
  fromCategoryId: budgets.find(item => item.name === fromCategory)?.categoryId ?? null,
  toCategoryId: null, fromAccountId: accounts.find(item => item.name === fromAccount)?.id ?? null,
  toAccountId: kind === 'income' ? 'checking' : kind === 'transfer' ? 'savings' : null,
  correctsTransactionId: null, incomeTreatment: kind === 'income' ? 'salary' : null,
  expectedIncomeMonth: kind === 'income' ? '2026-10' : null, hasCategoryCorrection: false, revision: 1, canEdit: true,
}));
const totalEur = money(accounts.reduce((sum, account) => sum + Number(account.balanceEur), 0));
export const rates = [
  { currency: 'EUR', date: '2026-10-18', unitsPerEur: '1' },
  { currency: 'USD', date: '2026-10-18', unitsPerEur: '1.1' },
  { currency: 'PHP', date: '2026-10-18', unitsPerEur: '62.5' },
];
const assignedRemainingEur = money(budgets.reduce((sum, category) => sum + Number(category.remainingEur), 0));
export const finance = {
  budgetCoverage: {
    actualCashEur: totalEur, assignedRemainingEur, unallocatedEur: money(Number(totalEur) - Number(assignedRemainingEur)),
    outstandingExpectedEur: '0.00', currentGapEur: money(Number(totalEur) - Number(assignedRemainingEur)),
    projectedGapEur: money(Number(totalEur) - Number(assignedRemainingEur)), frozen: false,
    expectations: [{ month: '2026-10', amount: '3200.00', currency: 'EUR', amountEur: '3200.00',
      receivedEur: '3200.00', receivedAmount: '3200.00', outstandingEur: '0.00', arrivalDay: 15, completedMonth: null }],
  },
  asOf: '2026-10-01', budgetMonth: '2026-10', canReallocate: true, scheduledAllocations: [],
  transactionCount: transactions.length, recordedIncomeEur: '3200.00',
  incomeExpectation: { amount: '3200.00', currency: 'EUR', amountEur: '3200.00', recordedAmount: '3200.00', outstandingEur: '0.00', arrivalDay: 15 },
  accountValuation: { asOf: '2026-10-18', totalEur, rates },
  recentTransactions: transactions, budgets, accounts,
  budgetHistory: ['2026-07', '2026-08', '2026-09', '2026-10'].map((month, index) => ({
    month, status: index === 3 ? 'open' : 'closed', carryoverProvisional: false,
    categories: index === 3 ? budgets : budgets.map(item => ({ ...item, expenseEur: money(Number(item.expenseEur) * (0.75 + index * 0.08)), netSpendingEur: money(Number(item.expenseEur) * (0.75 + index * 0.08)), remainingEur: money(Number(item.availableEur) - Number(item.expenseEur) * (0.75 + index * 0.08)) })),
  })),
  accountHistory: ['2026-07-31', '2026-08-31', '2026-09-30', '2026-10-18'].map((date, index) => {
    const historyAccounts = accounts.map(account => ({ accountId: account.id, name: account.name, currency: account.currency,
      balanceNative: money(Number(account.balanceNative) * (0.85 + index * 0.05)), balanceEur: money(Number(account.balanceEur) * (0.85 + index * 0.05)) }));
    return { date, totalEur: money(historyAccounts.reduce((sum, account) => sum + Number(account.balanceEur), 0)), accounts: historyAccounts };
  }),
  reconciliationStatus: { closingDate: '2026-09-30', batch: null },
};
export const reconciliation = {
  closingDate: '2026-09-30', observedAt: frozenTime, observedDate: '2026-10-18',
  totalEur, closingTotalEur: totalEur, reconciliation: null,
  rates: rates.map(rate => ({ ...rate, closingUnitsPerEur: rate.unitsPerEur, closingRateDate: rate.date })),
  accounts: accounts.map(account => {
    const activity = transactions.filter(row => row.fromAccountId === account.id || row.toAccountId === account.id)
      .map(row => ({ id: row.id, revision: row.revision, date: row.effectiveDate, occurredAt: row.occurredAt, description: row.description,
        netNative: money(row.toAccountId === account.id ? Number(row.toAmount) : -Number(row.amount)) }));
    const activityNative = activity.reduce((sum, row) => sum + Number(row.netNative), 0);
    return { id: account.id, name: account.name, type: account.accountType, currency: account.currency,
      balanceNative: account.balanceNative, closingBalanceNative: money(Number(account.balanceNative) - activityNative), activityNative: money(activityNative),
      existingAdjustmentNative: '0.00', ledgerRevision: account.ledgerRevision, transactions: activity };
  }),
};
reconciliation.closingTotalEur = money(reconciliation.accounts.reduce((sum, account) => sum + Number(account.closingBalanceNative) / Number(rates.find(rate => rate.currency === account.currency)!.unitsPerEur), 0));
