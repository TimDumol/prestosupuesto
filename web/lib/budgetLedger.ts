export type IncomeTreatment = 'salary' | 'unexpected' | 'refund' | 'legacy';
export type PlannedIncome = {
  month: string; amount: number; currency: string; amountEur: number;
  receivedEur: number; receivedAmount: number; outstandingEur: number; arrivalDay: number;
  completedMonth: string | null;
};
export type BudgetCoverage = {
  actualCashEur: number; assignedRemainingEur: number; unallocatedEur: number;
  outstandingExpectedEur: number; currentGapEur: number; projectedGapEur: number;
  expectations: PlannedIncome[]; frozen: boolean;
};
export type RawBudgetCoverage = {
  actualCashEur: string; assignedRemainingEur: string; unallocatedEur: string;
  outstandingExpectedEur: string; currentGapEur: string; projectedGapEur: string;
  expectations: (Omit<PlannedIncome, 'amount' | 'amountEur' | 'receivedEur' | 'receivedAmount' | 'outstandingEur'> & {
    amount: string; amountEur: string; receivedEur: string; receivedAmount: string; outstandingEur: string;
  })[]; frozen: boolean;
};
export function mapBudgetCoverage(source: RawBudgetCoverage | null | undefined): BudgetCoverage | null {
  if (!source) return null;
  return {
    actualCashEur: Number(source.actualCashEur), assignedRemainingEur: Number(source.assignedRemainingEur),
    unallocatedEur: Number(source.unallocatedEur), outstandingExpectedEur: Number(source.outstandingExpectedEur),
    currentGapEur: Number(source.currentGapEur), projectedGapEur: Number(source.projectedGapEur), frozen: source.frozen,
    expectations: source.expectations.map(e => ({ ...e, amount: Number(e.amount), amountEur: Number(e.amountEur),
      receivedEur: Number(e.receivedEur), receivedAmount: Number(e.receivedAmount), outstandingEur: Number(e.outstandingEur) })),
  };
}
