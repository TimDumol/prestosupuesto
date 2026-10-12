import type { GatewayTransaction, GatewayBudget, SubmitTransaction } from './contracts.ts';

export function possibleDuplicates(payload: SubmitTransaction, rows: GatewayTransaction[]) {
  const words = (text: string) => new Set(text.toLowerCase().normalize('NFKD').replace(/\p{M}/gu, '').match(/[\p{L}\p{N}]+/gu) ?? []);
  const description = words(payload.description);
  return rows.filter(row => {
    if (row.transactionId === payload.transactionId || row.kind !== payload.kind || row.date !== payload.date
      || row.currency !== payload.currency || row.amount === null || Math.round(row.amount * 100) !== Math.round(payload.amount * 100)
      || ((payload.kind === 'expense' || payload.kind === 'transfer') && row.fromAccount !== payload.fromAccount)
      || ((payload.kind === 'income' || payload.kind === 'transfer') && row.toAccount !== payload.toAccount)) return false;
    const other = words(row.description ?? '');
    const common = [...description].filter(word => other.has(word)).length;
    return description.size > 0 && common / Math.max(description.size, other.size) >= 0.5;
  });
}

export function balancingProposal(budgets: GatewayBudget[]) {
  const donors = budgets.filter(b => b.remainingEur != null && b.remainingEur > 0)
    .map(b => ({ name: b.name, cents: Math.round(b.remainingEur! * 100) })).sort((a, b) => b.cents - a.cents);
  const moves: { from: string; to: string; amount: number }[] = [];
  let uncovered = 0;
  for (const deficit of budgets.filter(b => b.remainingEur != null && b.remainingEur < 0)) {
    let need = -Math.round(deficit.remainingEur! * 100);
    for (const donor of donors) {
      const cents = Math.min(need, donor.cents);
      if (cents > 0) moves.push({ from: donor.name, to: deficit.name, amount: cents / 100 });
      donor.cents -= cents; need -= cents;
    }
    uncovered += need;
  }
  return { moves, uncovered: uncovered / 100 };
}
