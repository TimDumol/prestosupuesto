type Account = { name: string; currency: string };
export type PreviewInput = {
  kind: 'expense' | 'income' | 'transfer' | 'reallocate';
  amount: number; currency: string; toAmount?: number | null;
  fromAccount?: Account; toAccount?: Account;
  fromCategory?: string; toCategory?: string;
  incomeTreatment?: string; outstandingEur?: number;
  correction?: boolean; editing?: boolean;
};

const money = (amount: number, code: string) => new Intl.NumberFormat('en-IE', {
  style: 'currency', currency: code, maximumFractionDigits: 2,
}).format(amount);

/** Describe the entered transaction without guessing historical FX or edit deltas. */
export function transactionPreview(input: PreviewInput): string[] {
  if (!Number.isFinite(input.amount) || input.amount <= 0) return [];
  const { amount, currency, kind } = input;
  const value = money(amount, currency);
  const eur = currency === 'EUR' ? amount : null;
  const budgetValue = eur === null ? `the EUR value of ${value}` : money(eur, 'EUR');
  const accountValue = (account: Account) => account.currency === currency ? value : `${value} converted to ${account.currency}`;
  const lines: string[] = [];
  if (input.editing) lines.push('This replaces the original entry. Effects below describe the replacement, not the difference from the original.');
  if (kind === 'expense' && input.fromAccount) lines.push(`${input.fromAccount.name} decreases by ${accountValue(input.fromAccount)}.`);
  if (kind === 'income' && input.toAccount) lines.push(`${input.toAccount.name} increases by ${accountValue(input.toAccount)}.`);
  if (kind === 'transfer') {
    if (input.fromAccount) lines.push(`${input.fromAccount.name} decreases by ${accountValue(input.fromAccount)}.`);
    if (input.toAccount) {
      const crossCurrency = input.fromAccount?.currency !== input.toAccount.currency;
      lines.push(crossCurrency ? input.toAmount && input.toAmount > 0
        ? `${input.toAccount.name} increases by ${money(input.toAmount, input.toAccount.currency)}.`
        : `Enter the amount received in ${input.toAccount.currency}.`
        : `${input.toAccount.name} increases by ${accountValue(input.toAccount)}.`);
    }
    lines.push('Category budgets stay unchanged.');
  } else if (kind === 'reallocate') {
    if (input.fromCategory) lines.push(`${input.fromCategory} decreases by ${money(amount, 'EUR')}.`);
    if (input.toCategory) lines.push(`${input.toCategory} increases by ${money(amount, 'EUR')}.`);
    lines.push('Account balances stay unchanged.');
  } else if (input.correction) {
    lines.push('This is linked to an earlier entry. Its budget effect follows the original transaction’s correction rules.');
  } else if (kind === 'expense' && input.fromCategory) {
    lines.push(`${input.fromCategory} remaining budget decreases by ${budgetValue}.`);
  } else if (kind === 'income') {
    if (input.incomeTreatment === 'salary') {
      lines.push('Category allocations stay unchanged.');
      if (!input.editing && eur !== null && input.outstandingEur !== undefined) {
        const settled = Math.min(eur, Math.max(0, input.outstandingEur));
        lines.push(`Expected salary decreases by ${money(settled, 'EUR')}; Unallocated increases by ${money(eur - settled, 'EUR')}.`);
      } else lines.push('Settles the selected month’s expected salary; any excess goes to Unallocated.');
    } else if (input.incomeTreatment === 'unexpected') lines.push(`Unallocated increases by ${budgetValue}; category allocations stay unchanged.`);
    else if (input.toCategory) lines.push(`${input.toCategory} remaining budget increases by ${budgetValue}.`);
  }
  if (currency !== 'EUR' && kind !== 'reallocate') lines.push('EUR values and currency conversions use the rate for the transaction date when saved.');
  return lines;
}
