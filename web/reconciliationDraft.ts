export function reconciliationDraftKey(user: string, closingDate: string): string {
  return 'presto-reconciliation-draft:' + JSON.stringify([user, closingDate]);
}

export function readReconciliationDraft(storage: Storage, key: string): Record<string, string> {
  try {
    const value: unknown = JSON.parse(storage.getItem(key) ?? '{}');
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
    return Object.fromEntries(Object.entries(value).filter(([, balance]) => typeof balance === 'string'));
  } catch {
    return {};
  }
}

export function writeReconciliationDraft(storage: Storage, key: string, balances: Record<string, string>): boolean {
  try {
    if (Object.keys(balances).length) storage.setItem(key, JSON.stringify(balances));
    else storage.removeItem(key);
    return true;
  } catch {
    return false;
  }
}
