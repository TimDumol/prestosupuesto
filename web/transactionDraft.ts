import type { Currency, SubmitTransaction } from './contracts';

export type TransactionDraft = {
  operationId: string;
  baseRevision: string;
  kind: SubmitTransaction['kind'];
  incomeTreatment: 'salary' | 'unexpected' | 'refund' | 'legacy';
  expectedIncomeMonth: string;
  correctsTransactionId: string;
  date: string;
  description: string;
  amount: string;
  currency: Currency;
  toAmount: string;
  fromCategory: string;
  toCategory: string;
  fromAccount: string;
  toAccount: string;
};


export function transactionDraftKey(user: string, month: string, id = 'new') {
  return 'presto-transaction-draft:' + JSON.stringify([user, month, id]);
}
export function readTransactionDraft(key: string, fallback: TransactionDraft, storage?: Storage): TransactionDraft {
  try {
    const value: unknown = JSON.parse((storage ?? window.localStorage).getItem(key) ?? 'null');
    if (!value || typeof value !== 'object' || Array.isArray(value)) return fallback;
    const source = value as Record<string, unknown>;
    if (!Object.keys(fallback).every(key => typeof source[key] === 'string')
      || !['expense', 'income', 'transfer', 'reallocate'].includes(String(source.kind))
      || !['EUR', 'USD', 'PHP'].includes(String(source.currency))
      || !['salary', 'unexpected', 'refund', 'legacy'].includes(String(source.incomeTreatment))) return fallback;
    return Object.fromEntries(Object.keys(fallback).map(key => [key, source[key]])) as TransactionDraft;
  } catch { return fallback; }
}
export function writeTransactionDraft(key: string, draft: TransactionDraft | null, storage?: Storage): boolean {
  try {
    const target = storage ?? window.localStorage;
    if (draft) target.setItem(key, JSON.stringify(draft));
    else target.removeItem(key);
    return true;
  } catch { return false; }
}
