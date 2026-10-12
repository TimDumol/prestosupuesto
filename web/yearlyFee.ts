// Round up to whole cents so the planned installments cover the fee.
export function yearlyFeePlan(fee: number, saved: number, currentMonth: string, dueMonth = '') {
  if (!Number.isFinite(fee) || fee <= 0 || !Number.isFinite(saved)) return null;
  const monthIndex = (month: string) => {
    if (!/^\d{4}-(0[1-9]|1[0-2])$/.test(month)) return null;
    const [year, value] = month.split('-').map(Number);
    return year * 12 + value - 1;
  };
  const current = monthIndex(currentMonth);
  const due = dueMonth ? monthIndex(dueMonth) : null;
  if (current === null || (dueMonth && (due === null || due < current))) return null;
  const months = due === null ? 12 : due - current + 1;
  const feeCents = Math.round(fee * 100);
  const savedCents = due === null ? 0 : Math.round(saved * 100);
  const neededCents = Math.max(0, feeCents - savedCents);
  return { months, monthly: Math.ceil(neededCents / months) / 100, needed: neededCents / 100,
    saved: savedCents / 100, annualMonthly: Math.ceil(feeCents / 12) / 100 };
}
