export type TransactionKind = 'expense' | 'income' | 'transfer' | 'reallocate';

export type Category = {
  id: string;
  name: string;
  monthly: number;
  rollover: number;
  adjustment: number;
  spent: number;
  system?: 'source' | 'sink';
};

export type Account = {
  id: string;
  name: string;
  currency: Currency;
  kind: 'bank' | 'card' | 'cash' | 'savings';
};

export type Currency = 'EUR' | 'USD' | 'GBP';

export type Transaction = {
  id: string;
  kind: TransactionKind;
  description: string;
  amount: number;
  currency: Currency;
  dateLabel: string;
  categoryFrom?: string;
  categoryTo?: string;
  accountFrom?: string;
  accountTo?: string;
};

export const SYSTEM_INCOME = 'income';
export const SYSTEM_EXPENSE = 'expense';

export const initialCategories: Category[] = [
  { id: 'food', name: 'Food & groceries', monthly: 450, rollover: 48, adjustment: 0, spent: 185.6 },
  { id: 'transport', name: 'Transport', monthly: 120, rollover: 21, adjustment: 0, spent: 45 },
  { id: 'eating-out', name: 'Eating out', monthly: 120, rollover: 35, adjustment: 0, spent: 77.2 },
  { id: 'home', name: 'Home repairs', monthly: 100, rollover: 40, adjustment: 0, spent: 20 },
  { id: 'travel', name: 'Travel', monthly: 200, rollover: 430, adjustment: 0, spent: 50 },
  { id: 'buffer', name: 'General buffer', monthly: 150, rollover: 265, adjustment: 0, spent: 0 },
  { id: SYSTEM_INCOME, name: 'Income', monthly: 0, rollover: 0, adjustment: 0, spent: 0, system: 'source' },
  { id: SYSTEM_EXPENSE, name: 'Expense', monthly: 0, rollover: 0, adjustment: 0, spent: 0, system: 'sink' },
];

export const accounts: Account[] = [
  { id: 'checking', name: 'Checking', currency: 'EUR', kind: 'bank' },
  { id: 'visa', name: 'Visa', currency: 'EUR', kind: 'card' },
  { id: 'cash', name: 'Cash', currency: 'EUR', kind: 'cash' },
  { id: 'savings', name: 'Savings', currency: 'EUR', kind: 'savings' },
];

export const initialTransactions: Transaction[] = [
  {
    id: 'sample-1',
    kind: 'expense',
    description: 'Corner café',
    amount: 2.2,
    currency: 'EUR',
    dateLabel: 'Today · 08:14',
    categoryFrom: 'eating-out',
    categoryTo: SYSTEM_EXPENSE,
    accountFrom: 'cash',
  },
  {
    id: 'sample-2',
    kind: 'expense',
    description: 'Metro pass',
    amount: 1.5,
    currency: 'EUR',
    dateLabel: 'Today · 07:45',
    categoryFrom: 'transport',
    categoryTo: SYSTEM_EXPENSE,
    accountFrom: 'visa',
  },
  {
    id: 'sample-3',
    kind: 'income',
    description: 'August salary',
    amount: 3250,
    currency: 'EUR',
    dateLabel: 'Yesterday',
    categoryFrom: SYSTEM_INCOME,
    categoryTo: 'buffer',
    accountTo: 'checking',
  },
  {
    id: 'sample-4',
    kind: 'transfer',
    description: 'Savings transfer',
    amount: 500,
    currency: 'EUR',
    dateLabel: 'Yesterday',
    accountFrom: 'checking',
    accountTo: 'savings',
  },
  {
    id: 'sample-5',
    kind: 'reallocate',
    description: 'Top up home repairs',
    amount: 100,
    currency: 'EUR',
    dateLabel: 'Yesterday',
    categoryFrom: 'travel',
    categoryTo: 'home',
  },
];

export const descriptionSuggestions = [
  'Weekly groceries',
  'Corner café',
  'Metro pass',
  'Monthly salary',
];

export function available(category: Category) {
  return category.monthly + category.rollover + category.adjustment - category.spent;
}

export function money(value: number, currency: Currency = 'EUR') {
  const symbol = currency === 'EUR' ? '€' : currency === 'USD' ? '$' : '£';
  return `${symbol}${Math.abs(value).toLocaleString('en-US', {
    minimumFractionDigits: Number.isInteger(value) ? 0 : 2,
    maximumFractionDigits: 2,
  })}`;
}

export function categoryName(categories: Category[], id?: string) {
  return categories.find((item) => item.id === id)?.name ?? '—';
}

export function accountName(id?: string) {
  return accounts.find((item) => item.id === id)?.name ?? '—';
}
