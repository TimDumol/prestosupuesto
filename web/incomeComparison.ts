export function incomeComparison({ balance, budget, expected, recorded, month, arrivalDay, today, outstandingEur }: {
  balance: number; budget: number; expected: number; recorded: number;
  month: string; arrivalDay: number; today: string; outstandingEur?: number;
}) {
  const cents = (value: number) => Math.round(value * 100);
  const outstanding = outstandingEur ?? Math.max(0, cents(expected) - cents(recorded)) / 100;
  const [year, monthNumber] = month.split('-').map(Number);
  const lastDay = new Date(Date.UTC(year, monthNumber, 0)).getUTCDate();
  const dueDate = month + '-' + String(Math.min(arrivalDay, lastDay)).padStart(2, '0');
  return {
    outstanding,
    dueDate,
    overdue: outstanding > 0 && today > dueDate,
    difference: (cents(balance) - cents(budget)) / 100,
    adjustedDifference: (cents(balance) + cents(outstanding) - cents(budget)) / 100,
  };
}
