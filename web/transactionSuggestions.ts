import type { FinanceSnapshot, GatewayTransaction } from './contracts';
import type { TransactionDraft } from './transactionDraft';

export const normalize = (value: string) => value.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
function editDistance(a: string, b: string) {
  let previous = Array.from({ length: b.length + 1 }, (_, i) => i);
  let beforePrevious = previous;
  for (let i = 0; i < a.length; i++) {
    const current = [i + 1];
    for (let j = 0; j < b.length; j++) {
      current[j + 1] = Math.min(current[j] + 1, previous[j + 1] + 1, previous[j] + (a[i] === b[j] ? 0 : 1));
      if (i > 0 && j > 0 && a[i] === b[j - 1] && a[i - 1] === b[j]) current[j + 1] = Math.min(current[j + 1], beforePrevious[j - 1] + 1);
    }
    beforePrevious = previous; previous = current;
  }
  return previous[b.length];
}
export function descriptionSuggestions(query: string, rows: GatewayTransaction[], kind: string, excluded?: string | null) {
  const tokens = normalize(query).split(' ').filter(Boolean);
  const groups = new Map<string, { item: GatewayTransaction; count: number; score: number }>();
  for (const item of rows) {
    const key = normalize(item.description);
    if (item.kind !== kind || !key || (excluded && item.transactionId === excluded)) continue;
    if (!tokens.every(token => key.includes(token) || key.split(' ').some(word => token.length >= 3 && editDistance(word, token) <= (token.length >= 6 ? 2 : 1)))) continue;
    const score = key === normalize(query) ? 3 : key.startsWith(normalize(query)) ? 2 : 1;
    const old = groups.get(key);
    groups.set(key, { item: old && (old.item.date ?? '') > (item.date ?? '') ? old.item : item, count: (old?.count ?? 0) + 1, score });
  }
  return [...groups.values()].sort((a, b) => b.score - a.score || b.count - a.count || (b.item.date ?? '').localeCompare(a.item.date ?? '')).slice(0, 5).map(group => group.item);
}
export function mostUsedAccount(snapshot: FinanceSnapshot, category?: string) {
  const matching = snapshot.recentTransactions.filter(t => t.fromCategory === category || t.toCategory === category);
  const rows = category && matching.length ? matching : snapshot.recentTransactions;
  const counts = new Map<string, number>();
  for (const row of rows) for (const name of new Set([row.fromAccount, row.toAccount])) if (name) counts.set(name, (counts.get(name) ?? 0) + 1);
  return [...snapshot.accounts].sort((a, b) => (counts.get(b.name) ?? 0) - (counts.get(a.name) ?? 0))[0];
}
const hints = [
  ['cleaner cleaning housekeeping', 'clean home house household'],
  ['grocery groceries market supermarket', 'food grocer market'],
  ['cafe coffee dinner lunch restaurant', 'dining eating restaurant food'],
  ['bus metro parking taxi train uber', 'car transport travel'],
  ['doctor dentist medicine pharmacy', 'health medical'],
  ['electric internet phone utility water', 'bill home house utility'],
  ['flight hotel holiday trip', 'holiday travel vacation'],
];
export function suggestedCategory(description: string, snapshot: FinanceSnapshot) {
  const key = normalize(description);
  if (!key) return undefined;
  const previous = descriptionSuggestions(description, snapshot.recentTransactions, 'expense')[0];
  if (previous?.fromCategory && snapshot.budgets.some(b => b.name === previous.fromCategory && b.name !== 'Unallocated')) return previous.fromCategory;
  return snapshot.budgets.find(b => b.name !== 'Unallocated' && (normalize(b.name).split(' ').some(word => word.length > 2 && key.split(' ').includes(word)) || hints.some(([keywords, terms]) => keywords.split(' ').some(word => key.includes(word)) && terms.split(' ').some(word => normalize(b.name).includes(word)))))?.name;
}
export function applyDescriptionSuggestion(draft: TransactionDraft, item: GatewayTransaction, snapshot: FinanceSnapshot): TransactionDraft {
  const next = { ...draft, description: item.description.trim() };
  if (item.kind !== draft.kind) return draft;
  for (const field of ['fromAccount', 'toAccount'] as const) if (snapshot.accounts.some(a => a.name === item[field])) next[field] = item[field]!;
  for (const field of ['fromCategory', 'toCategory'] as const) if (snapshot.budgets.some(b => b.name === item[field] && b.name !== 'Unallocated')) next[field] = item[field]!;
  if (draft.kind === 'income') next.incomeTreatment = item.incomeTreatment ?? 'legacy';
  next.currency = draft.kind === 'reallocate' ? 'EUR' : snapshot.accounts.find(a => a.name === (draft.kind === 'income' ? next.toAccount : next.fromAccount))?.currency ?? draft.currency;
  if (draft.kind === 'transfer' && (next.fromAccount !== draft.fromAccount || next.toAccount !== draft.toAccount)) next.toAmount = '';
  return next;
}
