import { balancingProposal, possibleDuplicates } from './uxHelpers';
import { transactionPreview } from './lib/transactionPreview';
import { useCallback, useEffect, useRef, useState, useId } from 'react';
import type { FormEvent, InputHTMLAttributes, ReactNode } from 'react';
import DatePicker from 'react-datepicker';
import { Select, Table } from 'antd';
import { readTransactionDraft, transactionDraftKey, writeTransactionDraft } from './transactionDraft';
import type { TransactionDraft } from './transactionDraft';
import { monthCloseChecklist } from './monthCloseChecklist';
import { parseMoneyAmount as parseAmount } from './amountInput';
import { incomeComparison } from './incomeComparison';
import { yearlyFeePlan } from './yearlyFee';
import { readReconciliationDraft, reconciliationDraftKey, writeReconciliationDraft } from './reconciliationDraft';
import {
  cancelMonthlyBudget,
  createBudgetCategory,
  getTransactionHistory,
  getFinanceSnapshot,
  getTransactions,
  getReconciliationPreview,
  getServerSession,
  invalidateServerSession,
  reconcileAccounts,
  startPocketIdLogin,
  signOutFromServer,
  submitTransaction,
  editTransaction,
  deleteTransaction,
  saveExpectedIncome,
  saveMonthlyBudget,
  closeBudgetMonth,
  completeExpectedIncome,
} from './api';
import { AccountHistoryChart, AccountHistoryComparisonChart, BudgetComposition, BudgetSpendingTimeline, MonthlySpendingChart } from './charts';
import type { AccountHistoryComparisonSeries, AccountHistoryDatum, BudgetTimelineDatum } from './charts';
import type {
  FinanceSnapshot,
  Currency,
  ReconciliationPreview,
  ReconciliationBatch,
  GatewayBudgetHistoryCategory,
  GatewayTransaction,
  SubmitTransaction,
} from './contracts';

type PageKey = 'overview' | 'budgets' | 'history' | 'reconcile' | 'ledger';
function allowDraftNavigation() {
  return window.dispatchEvent(new Event('presto-before-navigation', { cancelable: true }));
}

type LedgerFilter = { category?: string; account?: string };
type PageProps = { snapshot: FinanceSnapshot; onDrillDown?: (filter: LedgerFilter) => void };
type BudgetSelectOption = {
  value: string;
  label: string;
  categoryName: string;
  detail: string;
  searchText: string;
};

const navigation: { key: PageKey; label: string; icon: string }[] = [
  { key: 'overview', label: 'Overview', icon: '◫' },
  { key: 'budgets', label: 'Budgets', icon: '◧' },
  { key: 'history', label: 'Account history', icon: '⌁' },
  { key: 'reconcile', label: 'Reconciliation', icon: '↔' },
  { key: 'ledger', label: 'Transactions', icon: '≡' },
];

const pagePaths: Record<PageKey, string> = {
  overview: '/',
  budgets: '/budgets',
  history: '/history',
  reconcile: '/reconciliation',
  ledger: '/transactions',
};

function pageFromPath(pathname: string): PageKey {
  const normalizedPath = pathname.replace(/\/+$/, '') || '/';
  return (Object.keys(pagePaths) as PageKey[]).find((key) => pagePaths[key] === normalizedPath) ?? 'overview';
}

function madridDateKey() {
  const parts = new Intl.DateTimeFormat('en-CA', { timeZone: 'Europe/Madrid', year: 'numeric', month: '2-digit', day: '2-digit' }).formatToParts(new Date());
  const value = (type: string) => parts.find(part => part.type === type)!.value;
  return `${value('year')}-${value('month')}-${value('day')}`;
}
function currentMonthKey() { return madridDateKey().slice(0, 7); }

function monthDate(month: string) {
  const [year, monthNumber] = month.split('-').map(Number);
  return new Date(year, monthNumber - 1, 1);
}

function lastDayOfMonth(month: string) {
  const parts = month.split('-').map(Number);
  const day = new Date(Date.UTC(parts[0], parts[1], 0)).getUTCDate();
  return month + '-' + String(day).padStart(2, '0');
}

function monthLabel(month: string, short = false) {
  const date = new Date(month + '-01T12:00:00');
  return date.toLocaleDateString('en-GB', { month: short ? 'short' : 'long', year: 'numeric' });
}

function dateLabel(date: string | null) {
  if (!date) return '—';
  const parsed = new Date(date + 'T12:00:00Z');
  return Number.isNaN(parsed.valueOf())
    ? date
    : parsed.toLocaleDateString('en-GB', { day: '2-digit', month: 'short', year: 'numeric', timeZone: 'UTC' });
}

function transactionDateLabel(item: { date: string | null; occurredAt?: string | null }) {
  const time = item.occurredAt ? new Date(item.occurredAt) : null;
  return dateLabel(item.date) + (time && !Number.isNaN(time.valueOf()) ? ' · ' + time.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' }) : '');
}

function numeric(value: unknown) {
  const number = Number(value);
  return Number.isFinite(number) ? number : 0;
}

function budgetBalance(value: unknown) {
  return Number(new Intl.NumberFormat('en-US', {
    useGrouping: false,
    maximumFractionDigits: 2,
  }).format(numeric(value))) || 0;
}

function currency(value: number | null | undefined, code = 'EUR') {
  const number = numeric(value);
  try {
    return new Intl.NumberFormat('en-IE', {
      style: 'currency',
      currency: code,
      maximumFractionDigits: 2,
    }).format(number);
  } catch {
    return code + ' ' + number.toFixed(2);
  }
}

function signedCurrency(value: number, code: string) {
  if (Math.abs(value) < 0.005) return currency(0, code);
  return (value > 0 ? '+' : '−') + currency(Math.abs(value), code);
}

type NumericInputProps = Omit<InputHTMLAttributes<HTMLInputElement>, 'type' | 'value' | 'onChange'> & {
  value: string;
  onChange: (value: string) => void;
};

function NumericInput({ value, onChange, onBlur, ...props }: NumericInputProps) {
  const parsed = parseAmount(value);
  const hasValue = value.trim().length > 0;
  const helpId = useId();
  return <span className="numeric-input-wrap">
    <input {...props} type="text" inputMode="decimal" value={value}
      aria-invalid={hasValue && parsed === null ? true : undefined}
      aria-describedby={[props['aria-describedby'], hasValue ? helpId : null].filter(Boolean).join(' ') || undefined}
      title="Use 1234.56, 1234,56, 1.234,56 or 1,234.56. A single separator is decimal. Formulas start with =."
      onChange={event => onChange(event.target.value)} onBlur={onBlur} />
    {hasValue ? <small id={helpId} className={parsed === null ? 'inline-error' : 'amount-preview'}>
      {parsed === null ? 'Use at most two decimal places, e.g. 1.234,56 or 1,234.56. A single separator is decimal.' : 'Interpreted as ' + new Intl.NumberFormat('en-IE', { maximumFractionDigits: 12 }).format(parsed)}
    </small> : null}
  </span>;
}

function monthlyBudgetAmount(value: string) {
  const trimmed = value.trim();
  const parsed = parseAmount(trimmed);
  if (parsed === null || parsed < 0) return null;
  return parsed;
}

function uuidV7() {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  let timestamp = Date.now();
  for (let index = 5; index >= 0; index -= 1) {
    bytes[index] = timestamp & 255;
    timestamp = Math.floor(timestamp / 256);
  }
  bytes[6] = (bytes[6] & 15) | 112;
  bytes[8] = (bytes[8] & 63) | 128;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
  return hex.slice(0, 8) + '-' + hex.slice(8, 12) + '-' + hex.slice(12, 16) + '-' + hex.slice(16, 20) + '-' + hex.slice(20);
}

function budgetUse(item: GatewayBudgetHistoryCategory, ignoreReallocation: boolean) {
  if (item.netSpendingEur != null) return item.netSpendingEur;
  return numeric(item.expenseEur) - (ignoreReallocation ? numeric(item.reallocationImpactEur) : 0);
}

function transactionKind(item: GatewayTransaction) {
  const label = { expense: 'Expense', income: 'Income', transfer: 'Transfer', reallocate: 'Budget move', reconciliation: 'Reconciliation' }[item.kind ?? 'expense'];
  if (item.kind && label) return label;
  if (item.fromAccount === 'Reallocation' && item.toAccount === 'Reallocation') return 'Budget move';
  if (item.fromCategory === 'Reconciliation' && item.toCategory === 'Reconciliation') return 'Reconciliation';
  if (item.fromCategory === 'Income' && item.fromAccount === 'Income') return 'Income';
  if (item.fromCategory === 'Balance Transfer' && item.toCategory === 'Balance Transfer') return 'Transfer';
  return 'Expense';
}

function LedgerLabel({ name, dimension }: { name: string; dimension: 'account' | 'category' }) {
  // Name-based colors stay stable across filtering, pagination, and reloads.
  let hash = 0;
  for (const character of dimension + ':' + name) hash = (Math.imul(hash, 31) + character.charCodeAt(0)) | 0;
  return <span className={'ledger-label ledger-label-tone-' + (Math.abs(hash) % 8)}>{name}</span>;
}

function transactionDimension(item: GatewayTransaction, dimension: 'account' | 'category') {
  const kind = transactionKind(item);
  let from: string | null = null;
  let to: string | null = null;
  if (dimension === 'account') {
    if (kind === 'Budget move') return <span className="ledger-dimension-empty">—</span>;
    if (kind === 'Income') from = item.toAccount;
    else {
      from = item.fromAccount;
      if (kind === 'Transfer') to = item.toAccount;
    }
  } else {
    if (kind === 'Transfer' || kind === 'Reconciliation') return <span className="ledger-dimension-empty">—</span>;
    if (kind === 'Income') from = item.toCategory || 'Uncategorised';
    else {
      from = item.fromCategory || 'Uncategorised';
      if (kind === 'Budget move') to = item.toCategory;
    }
  }
  const movement = kind === (dimension === 'account' ? 'Transfer' : 'Budget move');
  return <div className="ledger-dimension">
    {from ? <LedgerLabel name={from} dimension={dimension} /> : <span className="ledger-dimension-empty">—</span>}
    {movement ? <><span className="ledger-direction" aria-label="to">→</span>{to ? <LedgerLabel name={to} dimension={dimension} /> : <span className="ledger-dimension-empty">—</span>}</> : null}
  </div>;
}


function compareNullableNumbers(left: number | null | undefined, right: number | null | undefined) {
  if (left == null) return right == null ? 0 : -1;
  if (right == null) return 1;
  return left - right;
}


function Tag({ children, tone = 'neutral' }: { children: ReactNode; tone?: 'neutral' | 'good' | 'warn' | 'bad' | 'info' | 'budget' }) {
  return <span className={'tag tag-' + tone}>{children}</span>;
}

function Panel({ children, className = '' }: { children: ReactNode; className?: string }) {
  return <section className={'panel ' + className}>{children}</section>;
}

function SectionHeading({ eyebrow, title, detail, action }: {
  eyebrow?: string;
  title: string;
  detail?: string;
  action?: ReactNode;
}) {
  return (
    <div className="section-heading">
      <div>
        {eyebrow ? <div className="eyebrow">{eyebrow}</div> : null}
        <h2>{title}</h2>
        {detail ? <p>{detail}</p> : null}
      </div>
      {action ? <div className="section-action">{action}</div> : null}
    </div>
  );
}

function EmptyState({ title, detail }: { title: string; detail: string }) {
  return <div className="empty-state"><span className="empty-mark">✳</span><strong>{title}</strong><p>{detail}</p></div>;
}

function Metrics({ snapshot, ignoreReallocation }: PageProps & { ignoreReallocation: boolean }) {
  const budget = snapshot.budgets;
  const history = snapshot.budgetHistory?.find((item) => item.month === snapshot.budgetMonth);
  const spent = history
    ? history.categories.reduce((total, item) => total + budgetUse(item, ignoreReallocation), 0)
    : budget.reduce((total, item) => total + numeric(item.expenseEur), 0);
  const plan = budget.reduce((total, item) => total + numeric(item.monthlyEur), 0);
  const available = budget.reduce((total, item) => total + numeric(item.remainingEur), 0);
  const coverage = snapshot.budgetCoverage;
  const balance = coverage?.actualCashEur ?? snapshot.accountValuation.totalEur;
  const overBudget = budget.filter((item) => budgetBalance(item.remainingEur) < 0).length;
  const reconciliationStatus = snapshot.reconciliationStatus;
  const reconciliation = reconciliationStatus.batch;
  const closeMonth = reconciliationStatus.closingDate.slice(0, 7);

  return (
    <div className="metric-grid">
      <article className="metric-card metric-highlight">
        <div className="metric-label">Spent this month <span>↘</span></div>
        <strong>{currency(spent)}</strong>
        <div className="metric-foot">{currency(plan)} planned across live budgets</div>
      </article>
      <article className="metric-card">
        <div className="metric-label">Remaining budget <span>◷</span></div>
        <strong className={available < 0 ? 'value-negative' : ''}>{currency(available)}</strong>
        <div className="metric-foot">{overBudget ? overBudget + ' categor' + (overBudget === 1 ? 'y below' : 'ies below') + ' zero' : 'All category balances are positive'}</div>
      </article>
      <article className="metric-card">
        <div className="metric-label">Account balance <span>⌂</span></div>
        <strong>{currency(balance)}</strong>
        <div className="metric-foot">Combined in EUR at the selected month’s rates</div>
      </article>
      <article className="metric-card">
        <div className="metric-label">Last reconciliation <span>↔</span></div>
        <strong>{reconciliation ? reconciliation.isCurrent === false ? 'Refresh needed' : reconciliation.isComplete ? 'Finished' : 'Partial' : '—'}</strong>
        <div className="metric-foot">{reconciliation
          ? reconciliation.isComplete
            ? monthLabel(reconciliation.effectiveDate.slice(0, 7)) + ' successfully reconciled'
            : monthLabel(reconciliation.effectiveDate.slice(0, 7)) + ' · ' + reconciliation.reconciledAccountCount + ' of ' + reconciliation.totalAccountCount + ' accounts'
          : monthLabel(closeMonth) + ' not reconciled yet'}</div>
      </article>
    </div>
  );
}

function ExchangeRateNote({ snapshot }: PageProps) {
  const rates = snapshot.accountValuation.rates.filter((rate) => rate.currency !== 'EUR');
  return <p className="funding-detail exchange-rate-note">
    {snapshot.budgetMonth === currentMonthKey() ? 'Latest available rates as of ' : 'Month-end rates as of '}{dateLabel(snapshot.accountValuation.asOf)}.
    {rates.map((rate) => <span key={rate.currency}> {rate.currency}: {rate.unitsPerEur.toLocaleString('en-GB', { maximumFractionDigits: 6 })} per EUR ({dateLabel(rate.date)}).</span>)}
  </p>;
}

function IncomeComparisonPanel({ snapshot, onRefresh }: PageProps & { onRefresh: () => Promise<boolean> }) {
  const expectation = snapshot.incomeExpectation;
  const [editing, setEditing] = useState(false);
  const [amount, setAmount] = useState('');
  const [day, setDay] = useState('7');
  const [incomeCurrency, setIncomeCurrency] = useState<Currency>('USD');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    setAmount(String(expectation.amount));
    setIncomeCurrency(expectation.currency);
    setDay(String(expectation.arrivalDay));
    setEditing(false);
    setError('');
  }, [snapshot.budgetMonth, expectation.amount, expectation.currency, expectation.arrivalDay]);
  const now = new Date();
  const today = madridDateKey();
  const coverage = snapshot.budgetCoverage;
  const balance = coverage?.actualCashEur ?? snapshot.accountValuation.totalEur;
  const budget = coverage?.assignedRemainingEur ?? snapshot.budgets.reduce((total, item) => total + numeric(item.remainingEur), 0);
  const comparison = incomeComparison({ balance, budget, expected: expectation.amountEur,
    recorded: snapshot.recordedIncomeEur, month: snapshot.budgetMonth,
    arrivalDay: expectation.arrivalDay, today, outstandingEur: coverage?.outstandingExpectedEur ?? expectation.outstandingEur });

  const awaitingFunding = Math.min(Math.max(0, -comparison.difference), comparison.outstanding);

  async function save() {
    const parsedAmount = parseAmount(amount);
    const parsedDay = parseAmount(day);
    if (parsedAmount === null || parsedAmount < 0 || parsedDay === null || !Number.isInteger(parsedDay) || parsedDay < 1 || parsedDay > 31) {
      setError('Enter a non-negative amount and an arrival day from 1 to 31.');
      return;
    }
    setSaving(true);
    setError('');
    try {
      await saveExpectedIncome(snapshot.budgetMonth, parsedAmount, incomeCurrency, parsedDay);
      if (await onRefresh()) setEditing(false);
      else setError('Income expectation saved. Refresh to update the comparison.');
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  }

  return <Panel className="income-comparison">
    <SectionHeading eyebrow="BUDGET FUNDING" title="Account balance vs budget"
      detail="Compare available cash with the total remaining budget."
      action={<button className="text-button" type="button" disabled={saving || snapshot.budgetMonth !== currentMonthKey()} onClick={() => { setAmount(String(expectation.amount)); setIncomeCurrency(expectation.currency); setDay(String(expectation.arrivalDay)); setEditing(!editing); setError(''); }}>{editing ? 'Cancel' : 'Edit expected income'}</button>} />
    <div className="funding-summary">
      <span>{(coverage?.currentGapEur ?? comparison.difference) < 0 ? 'Cash shortfall today' : 'Cash above assigned budgets'}</span>
      <strong className={(coverage?.currentGapEur ?? comparison.difference) < 0 ? 'value-negative' : 'value-positive'}>{currency(Math.abs(coverage?.currentGapEur ?? comparison.difference))}</strong>
      {comparison.outstanding > 0 ? <small>{currency(comparison.outstanding)} income still expected{comparison.overdue ? ' · overdue' : ''}</small> : null}
    </div>
    <details className="funding-details"><summary>Funding details and calculations</summary>
    <ExchangeRateNote snapshot={snapshot} />
    <div className="funding-grid">
      <div><span>Account balance · actual cash</span><strong>{currency(balance)}</strong></div>
      <div><span>Assigned category budgets</span><strong>{currency(coverage?.assignedRemainingEur ?? budget)}</strong></div>
      {coverage ? <div><span>Unallocated · planned funds</span><strong className={coverage.unallocatedEur < 0 ? 'value-negative' : ''}>{signedCurrency(coverage.unallocatedEur, 'EUR')}</strong></div> : null}
      <div><span>Difference before expected income</span><strong className={(coverage?.currentGapEur ?? comparison.difference) < 0 ? 'value-negative' : ''}>{signedCurrency(coverage?.currentGapEur ?? comparison.difference, 'EUR')}</strong></div>
      {expectation ? <>
        <div><span>Income still expected</span><strong>{currency(comparison.outstanding)}</strong></div>
        <div><span>Difference after expected income</span><strong className={(coverage?.projectedGapEur ?? comparison.adjustedDifference) < 0 ? 'value-negative' : 'value-positive'}>{signedCurrency(coverage?.projectedGapEur ?? comparison.adjustedDifference, 'EUR')}</strong></div>
      </> : null}
    </div>
    {expectation ? <>
      <p className={'funding-status' + (comparison.overdue ? ' funding-status-overdue' : '')} role="status">
        {comparison.outstanding > 0
          ? comparison.overdue
            ? currency(comparison.outstanding) + ' of expected income hasn’t been recorded. Expected around ' + dateLabel(comparison.dueDate) + '.'
            : awaitingFunding > 0
              ? currency(awaitingFunding) + ' of this month’s budget awaits income, expected around ' + dateLabel(comparison.dueDate) + '.'
              : currency(comparison.outstanding) + ' income is still expected around ' + dateLabel(comparison.dueDate) + '.'
          : expectation.amountEur > 0 ? 'This month’s expected income has been recorded.' : 'No income is expected this month.'}
      </p>
      <p className="funding-detail">{currency(expectation.recordedAmount, expectation.currency)} income recorded this month against {currency(expectation.amount, expectation.currency)} expected ({currency(expectation.amountEur)} at the selected rates). Expected income is not available cash.</p>
    </> : <p className="funding-detail">Set expected monthly income to account for the gap before it arrives.</p>}
    {coverage ? <>
      <p className="funding-detail">{coverage.frozen ? 'Figures frozen at month close. ' : ''}Unallocated includes anticipated funds. Funding gaps are informational; adjust category allocations explicitly.</p>
      {coverage.expectations.filter(e => e.outstandingEur > 0 || e.completedMonth).map(e => <div key={e.month} className="funding-detail">
        <strong>{monthLabel(e.month)} salary</strong> · {currency(e.receivedEur)} received · {currency(e.outstandingEur)} outstanding{e.completedMonth ? ' · completed in ' + monthLabel(e.completedMonth) : ''}
        {!coverage.frozen && e.outstandingEur > 0 ? <button type="button" className="text-button" disabled={saving} onClick={async () => {
          if (!window.confirm(`Complete ${monthLabel(e.month)} income? This removes ${currency(e.outstandingEur)} of expected funding from Unallocated. Assigned budgets stay as planned.`)) return;
          setSaving(true); setError('');
          try { await completeExpectedIncome(e.month); await onRefresh(); }
          catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
          finally { setSaving(false); }
        }}>Mark complete</button> : null}
      </div>)}
    </> : null}
    </details>
    {editing ? <form className="income-settings" onSubmit={(event) => { event.preventDefault(); void save(); }}>
      <label><span>Expected monthly income</span><NumericInput value={amount} disabled={saving} onChange={setAmount} /></label>
      <label><span>Income currency</span><select value={incomeCurrency} disabled={saving} onChange={(event) => setIncomeCurrency(event.target.value as Currency)}><option>USD</option><option>EUR</option><option>PHP</option></select></label>
      <label><span>Expected arrival day</span><NumericInput value={day} disabled={saving} onChange={setDay} /></label>
      <button className="button button-primary" type="submit" disabled={saving}>{saving ? 'Saving…' : 'Save expected income'}</button>
      <p>Applies from {monthLabel(snapshot.budgetMonth)} onward, until changed. For shorter months, the arrival date is the last day of the month.</p>
    </form> : null}
    {error ? <p className="inline-error" role="alert">{error}</p> : null}
  </Panel>;
}

function TrendPanel({ snapshot, onSelectMonth, ignoreReallocation }: PageProps & {
  onSelectMonth: (month: string) => void;
  ignoreReallocation: boolean;
}) {
  const months = [...(snapshot.budgetHistory ?? [])].sort((left, right) => left.month.localeCompare(right.month)).slice(-8);
  const totals = months.map((month) => ({
    month: month.month,
    amount: month.categories.reduce((total, item) => total + Math.max(0, budgetUse(item, ignoreReallocation)), 0),
  }));
  return (
    <Panel className="trend-panel">
      <div className="panel-heading">
        <div><div className="eyebrow">SPENDING TREND</div><h2>Month by month</h2></div>
        <span className="panel-caption">Spending in EUR</span>
      </div>
      {totals.length ? <MonthlySpendingChart
        data={totals}
        activeMonth={snapshot.budgetMonth}
        onSelectMonth={onSelectMonth}
        formatMonth={monthLabel}
        formatCurrency={currency}
        formatCompactCurrency={compactCurrency}
      /> : <EmptyState title="No monthly history yet" detail="Monthly spending will appear here as activity is recorded." />}
    </Panel>
  );
}

function FocusPanel({ snapshot, onNavigate }: PageProps & { onNavigate: (page: PageKey) => void }) {
  const deficits = snapshot.budgets.filter((item) => budgetBalance(item.remainingEur) < 0).length;
  const reconciliationStatus = snapshot.reconciliationStatus;
  const reconciliation = reconciliationStatus.batch;
  const closeMonth = reconciliationStatus.closingDate.slice(0, 7);
  const message = deficits
    ? deficits + ' budget categor' + (deficits === 1 ? 'y is' : 'ies are') + ' below zero. Review available funds and move surplus where it is needed.'
    : reconciliation?.isCurrent === false
      ? 'Account activity changed after the saved reconciliation. Review and reconcile again before closing.'
      : reconciliation?.isComplete
      ? monthLabel(reconciliation.effectiveDate.slice(0, 7)) + ' was successfully reconciled. All ' + reconciliation.reconciledAccountCount + ' active accounts are included in the saved run.'
      : reconciliation
        ? monthLabel(reconciliation.effectiveDate.slice(0, 7)) + ' has a saved reconciliation for ' + reconciliation.reconciledAccountCount + ' of ' + reconciliation.totalAccountCount + ' active accounts.'
        : monthLabel(closeMonth) + ' has not been reconciled yet. Review the ledger before reconciling.';

  return (
    <Panel className="focus-panel">
      <div className="focus-icon">✦</div>
      <div className="eyebrow">MONTH-END CHECK</div>
      <h2>{deficits ? 'Budget balancing' : reconciliation?.isComplete ? monthLabel(reconciliation.effectiveDate.slice(0, 7)) + ' reconciled' : 'Reconcile accounts'}</h2>
      <p>{message}</p>
      <button className="text-button" type="button" onClick={() => onNavigate(deficits ? 'budgets' : 'reconcile')}>
        {deficits ? 'Review budgets' : 'Open reconciliation'} <span>↗</span>
      </button>
    </Panel>
  );
}

function CategoryTable({ snapshot, ignoreReallocation, onDrillDown }: PageProps & { ignoreReallocation: boolean }) {
  const month = snapshot.budgetHistory?.find((item) => item.month === snapshot.budgetMonth);
  const history = new Map((month?.categories ?? []).map((item) => [item.name, item] as const));
  const budgets = snapshot.budgets.map(item => {
    const historical = history.get(item.name);
    const spent = historical ? budgetUse(historical, ignoreReallocation) : numeric(item.expenseEur);
    const planned = numeric(item.monthlyEur);
    const remaining = budgetBalance(item.remainingEur);
    const percent = planned > 0 ? Math.max(0, Math.min(100, Math.max(0, spent) / planned * 100)) : 0;
    const isOverPlan = planned > 0 && spent > planned;
    return { ...item, spent, planned, remaining, percent, isOverPlan,
      status: remaining < 0 ? 'Below zero' : isOverPlan ? 'Over plan' : 'On track' };
  });

  return <Table
    className="sortable-table"
    rowKey="name"
    dataSource={budgets}
    pagination={false}
    scroll={{ x: 'max-content' }}
    locale={{ emptyText: <EmptyState title="No budgets returned" detail="Review your category opening balances and monthly plan." /> }}
    columns={[
      { title: 'Category', key: 'category', sorter: (a, b) => a.name.localeCompare(b.name), render: (_, item) => <button type="button" className="text-button category-name" onClick={() => onDrillDown?.({ category: item.name })}><i className="category-dot" />{item.name} ↗</button> },
      { title: 'Remaining', key: 'remaining', defaultSortOrder: 'ascend', sorter: (a, b) => a.remaining - b.remaining, render: (_, item) => <span className={item.remaining < 0 ? 'value-negative' : 'value-positive'}>{currency(item.remaining)}</span> },
      { title: 'Spent', key: 'spent', sorter: (a, b) => a.spent - b.spent, render: (_, item) => currency(item.spent) },
      { title: 'Monthly plan', key: 'planned', sorter: (a, b) => a.planned - b.planned, render: (_, item) => currency(item.planned) },
      { title: 'Carryover', key: 'carryover', sorter: (a, b) => numeric(a.carryoverEur) - numeric(b.carryoverEur), render: (_, item) => currency(item.carryoverEur) },
      { title: 'Adjustments', key: 'adjustments', sorter: (a, b) => numeric(a.adjustmentEur) - numeric(b.adjustmentEur), render: (_, item) => currency(item.adjustmentEur) },
      { title: 'Progress', key: 'progress', sorter: (a, b) => a.percent - b.percent, render: (_, item) => <div className="progress-cell"><span className="progress-track"><span className={'progress-fill ' + (item.isOverPlan ? 'progress-fill-warn' : '')} style={{ width: item.percent + '%' }} /></span><small>{Math.round(item.percent)}%</small></div> },
      { title: 'Status', key: 'status', sorter: (a, b) => a.status.localeCompare(b.status), render: (_, item) => <Tag tone={item.remaining < 0 ? 'bad' : item.isOverPlan ? 'warn' : 'good'}>{item.status}</Tag> },
    ]}
  />;
}

function AccountStrip({ snapshot, onDrillDown }: PageProps) {
  return (
    <div className="account-strip">
      {snapshot.accounts.map((account) => {
        const difference = numeric(account.reconciliationNative);
        return (
          <div className="account-mini" key={account.name}>
            <span className="account-symbol">{account.type?.toLowerCase().includes('card') ? '▱' : account.type?.toLowerCase().includes('cash') ? '◌' : '⌂'}</span>
            <div className="account-mini-copy"><button type="button" className="text-button" onClick={() => onDrillDown?.({ account: account.name })}>{account.name} ↗</button><small>{account.currency} · {Math.abs(difference) > 0.01 ? 'Recorded adjustment ' + signedCurrency(difference, account.currency) : 'No adjustment recorded'}</small></div>
            <b>{currency(account.balanceNative, account.currency)}</b>
          </div>
        );
      })}
      {!snapshot.accounts.length ? <EmptyState title="No active accounts" detail="Your active accounts will appear here." /> : null}
    </div>
  );
}

function OverviewPage({ snapshot, onSelectMonth, onNavigate, ignoreReallocation, onToggleReallocation, onRefresh, onDrillDown }: PageProps & {
  onSelectMonth: (month: string) => void;
  onRefresh: () => Promise<boolean>;
  onNavigate: (page: PageKey) => void;
  ignoreReallocation: boolean;
  onToggleReallocation: () => void;
}) {
  return (
    <>
      <div className="page-intro">
        <div><div className="eyebrow">FINANCIAL REVIEW</div><h1>{monthLabel(snapshot.budgetMonth)} at a glance</h1><p>Your household budgets, balances and recent activity.</p></div>
        <label className="check-switch"><input type="checkbox" checked={ignoreReallocation} onChange={onToggleReallocation} /><span>Adjust for budget moves</span></label>
      </div>
      <Metrics snapshot={snapshot} ignoreReallocation={ignoreReallocation} />
      <div className="overview-panels">
        <div className="overview-focus"><FocusPanel snapshot={snapshot} onNavigate={onNavigate} /></div>
        <div className="overview-category"><Panel>
          <SectionHeading eyebrow="CATEGORY BREAKDOWN" title="Where the budget stands" detail="Ordered by available balance so categories needing attention appear first." action={<button className="text-button" type="button" onClick={() => onNavigate('budgets')}>All budgets ↗</button>} />
          <CategoryTable snapshot={snapshot} ignoreReallocation={ignoreReallocation} onDrillDown={onDrillDown} />
        </Panel></div>
        <div className="overview-funding"><IncomeComparisonPanel snapshot={snapshot} onRefresh={onRefresh} /></div>
        <div className="overview-trend"><TrendPanel snapshot={snapshot} onSelectMonth={onSelectMonth} ignoreReallocation={ignoreReallocation} /></div>
        <div className="overview-accounts"><Panel className="accounts-panel">
          <SectionHeading eyebrow="ACCOUNTS" title="Balances across your accounts" detail="Book balance and any recorded reconciliation difference." action={<><button className="text-button" type="button" onClick={() => onNavigate('history')}>View history ↗</button><button className="text-button" type="button" onClick={() => onNavigate('reconcile')}>Reconcile ↗</button></>} />
          <AccountStrip snapshot={snapshot} onDrillDown={onDrillDown} />
        </Panel></div>
      </div>
    </>
  );
}

function BudgetMonthLifecycle({ snapshot, onRefresh, onMutation, mutation, onNavigate }: {
  snapshot: FinanceSnapshot; onRefresh: () => Promise<boolean>;
  onNavigate: (page: PageKey, month?: string) => void;
  onMutation: (value: string) => void; mutation: string;
}) {
  const month = snapshot.budgetHistory?.find(item => item.month === snapshot.budgetMonth);
  const [reviewing, setReviewing] = useState(false);
  const [message, setMessage] = useState('');
  useEffect(() => { setReviewing(false); setMessage(''); }, [snapshot.budgetMonth]);
  const currentMonth = currentMonthKey();
  const canReviewClose = month?.status === 'open' && snapshot.budgetMonth < currentMonth;
  const checks = monthCloseChecklist(snapshot, currentMonth);
  const ready = checks.every(check => !check.required || check.ready);
  const warnings = checks.some(check => !check.required && !check.ready);
  const [acknowledged, setAcknowledged] = useState(false);
  useEffect(() => { setAcknowledged(false); }, [snapshot.budgetMonth, snapshot.generatedAt]);
  async function close() {
    if (!ready || (warnings && !acknowledged)) return;
    onMutation('Closing budget month…'); setMessage('');
    try {
      await closeBudgetMonth(snapshot.budgetMonth);
      setReviewing(false);
      if (!await onRefresh()) setMessage('Month closed. Refresh to load its final balances.');
    } catch (error) { setMessage(error instanceof Error ? error.message : 'Could not close month.'); }
    finally { onMutation(''); }
  }
  return <Panel>
    <SectionHeading eyebrow="MONTH STATUS" title={month?.status === 'open' ? 'Open budget' : month?.status === 'closed' ? 'Closed budget' : 'Imported history'}
      detail={month?.status === 'open' ? 'Balances update from transactions and budget decisions.' : 'These historical balances are fixed.'} />
    {canReviewClose ? <ul className="close-checklist" aria-label="Month close checklist">
      {checks.map(check => <li key={check.id}>
        <span aria-hidden="true">{check.ready ? '✓' : check.required ? '○' : '!'}</span>
        <div><strong>{check.label} <small>{check.required ? 'Required' : 'Review'}</small></strong><p>{check.detail}</p></div>
        {!check.ready && check.id !== 'period' ? <button className="text-button" type="button" disabled={!!mutation} onClick={() => {
          if (check.id === 'deficits' || check.id === 'earlier' && !check.month) document.getElementById('budget-move-source')?.focus();
          else onNavigate(check.page, check.month);
        }}>Review ↗</button> : null}
      </li>)}
    </ul> : null}
    {canReviewClose && snapshot.reconciliationStatus.batch?.isCurrent === false ? <p role="status">Account activity changed since reconciliation. Reconcile again before closing.</p> : null}
    {month?.carryoverProvisional ? <p role="status">Earlier months are awaiting close. Their edits can change this month’s carryover.</p> : null}
    {canReviewClose ? <>
      <p>Reconcile accounts and finish budget moves before closing. Months close in order.</p>
      {!ready ? <p>Finish the required checks above to enable closing.</p> : warnings && !acknowledged && reviewing ? <p>Acknowledge the financial review below to enable closing.</p> : null}
      {reviewing ? <div><p>Closing {monthLabel(snapshot.budgetMonth)} freezes its reported balances. Later corrections will be recorded in an open month.</p>
        {warnings ? <label className="check-switch"><input type="checkbox" checked={acknowledged} disabled={!!mutation} onChange={event => setAcknowledged(event.target.checked)} />I reviewed the outstanding income and category balances and want to close with these values.</label> : null}
        <button type="button" className="button button-primary" disabled={!!mutation || !ready || warnings && !acknowledged} onClick={() => void close()}>Freeze {monthLabel(snapshot.budgetMonth)} balances</button>
        <button type="button" className="text-button" disabled={!!mutation} onClick={() => setReviewing(false)}>Cancel</button></div>
        : <button type="button" className="text-button" disabled={!!mutation || !ready} onClick={() => setReviewing(true)}>Review month close</button>}
    </> : null}
    {message ? <p role="status">{message}</p> : null}
  </Panel>;
}

function YearlyFeeCalculator({ saved = 0, startMonth = currentMonthKey(), disabled, automatic = false, onApply }: {
  saved?: number; startMonth?: string; disabled: boolean; automatic?: boolean; onApply: (amount: string) => void;
}) {
  const [fee, setFee] = useState('');
  const [dueMonth, setDueMonth] = useState('');
  const [applied, setApplied] = useState(false);
  const parsed = monthlyBudgetAmount(fee);
  const plan = parsed === null ? null : yearlyFeePlan(parsed, saved, startMonth, dueMonth);
  useEffect(() => { setApplied(false); }, [fee, dueMonth, saved, startMonth]);
  useEffect(() => { if (automatic) onApply(plan ? String(plan.monthly) : ''); }, [automatic, plan?.monthly, onApply]);
  return <div className="yearly-fee-calculator">
    <div className="budget-field-grid">
      <label><span>Yearly fee · EUR</span><NumericInput value={fee} disabled={disabled} onChange={setFee} placeholder="e.g. 120" /></label>
      <label><span>Next payment month (optional)</span><input type="month" min={startMonth} value={dueMonth} disabled={disabled} onChange={event => setDueMonth(event.target.value)} /></label>
    </div>
    {plan ? <div className="yearly-fee-result">
      <div className="fee-visual-heading"><span className="budget-illustration" aria-hidden="true">◷</span><div><span className="eyebrow">MONTHLY SAVING</span><strong>{currency(plan.monthly)}<small> / month</small></strong></div><div className="fee-target"><span>Yearly fee</span><strong>{currency(parsed)}</strong></div></div>
      <div className="fee-installments" aria-hidden="true">{Array.from({ length: Math.min(plan.months, 24) }, (_, index) => <i key={index} />)}</div>
      <div className="fee-visual-labels"><span>{plan.months} monthly allocation{plan.months === 1 ? '' : 's'}</span><span>{dueMonth ? 'Ready by ' + monthLabel(dueMonth) : 'One year of saving'}</span></div>
      {dueMonth ? <><div className="fee-savings-breakdown"><div><span>Available before this allocation</span><strong>{currency(plan.saved)}</strong></div><div><span>Still to save</span><strong>{currency(plan.needed)}</strong></div></div><p>Includes the payment month. If the fee is due before that month’s allocation, choose the previous month.</p><p>After payment: {currency(plan.annualMonthly)} / month saves for the following year.</p></> : <p>Unspent funds carry forward, building up the balance for your yearly payment.</p>}
      {automatic ? <span className="allocation-applied">✓ Monthly allocation updated above</span> : <button type="button" className="button button-secondary" disabled={disabled} onClick={() => { onApply(String(plan.monthly)); setApplied(true); }}>{applied ? 'Monthly amount applied' : 'Use ' + currency(plan.monthly) + ' / month'}</button>}
    </div> : <p>{fee || dueMonth ? 'Enter a positive yearly fee and choose this month or a later payment month.' : 'Enter the fee to calculate a monthly amount. Add a payment month if the next fee is less than a year away.'}</p>}
    <p className="funding-detail">Only the monthly allocation is saved; the payment date is a planning aid. Review the allocation after paying the fee.</p>
  </div>;
}

function NewBudgetCategory({ snapshot, onRefresh, mutation, onMutation }: PageProps & {
  onRefresh: () => Promise<boolean>; mutation: string; onMutation: (value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');
  const [yearly, setYearly] = useState(false);
  const [amount, setAmount] = useState('0');
  const [message, setMessage] = useState('');
  const [saving, setSaving] = useState(false);
  const duplicate = snapshot.budgets.some(item => item.name.trim().toLocaleLowerCase() === name.trim().toLocaleLowerCase());
  const valid = name.trim().length > 0 && name.trim().length <= 80 && !duplicate && monthlyBudgetAmount(amount) !== null;
  async function create(event: FormEvent) {
    event.preventDefault();
    if (!valid || mutation || saving) return;
    setSaving(true); onMutation('Creating category…'); setMessage('');
    try {
      await createBudgetCategory(name.trim(), String(monthlyBudgetAmount(amount)));
      const refreshed = await onRefresh();
      setOpen(false); setName(''); setAmount('0'); setYearly(false);
      setMessage(refreshed ? 'Category created.' : 'Category created. Refresh to load it.');
    } catch (error) { setMessage(error instanceof Error ? error.message : 'Could not create category.'); }
    finally { setSaving(false); onMutation(''); }
  }
  return <div className="new-budget-category">
    <button type="button" className="button button-secondary" disabled={!!mutation || saving} onClick={() => { setOpen(!open); setMessage(''); }}>{open ? 'Cancel new category' : '+ New category'}</button>
    {open ? <form className="new-category-form" onSubmit={event => void create(event)}>
      <div className="budget-field-grid">
      <label><span>Category name</span><input autoFocus required maxLength={80} value={name} disabled={saving} onChange={event => setName(event.target.value)} placeholder="e.g. Home maintenance" /></label>
      <label><span>Monthly allocation · EUR</span><NumericInput value={amount} disabled={saving} readOnly={yearly} onChange={setAmount} /></label>
      </div>
      <div className="budget-use-cases" role="group" aria-label="Budgeting approach">
        <button type="button" aria-pressed={!yearly} disabled={saving} onClick={() => setYearly(false)}><span aria-hidden="true">↻</span><strong>Monthly spending</strong><small>Groceries, rent, everyday life</small></button>
        <button type="button" aria-pressed={yearly} disabled={saving} onClick={() => setYearly(true)}><span aria-hidden="true">◷</span><strong>Save for a yearly fee</strong><small>Insurance, memberships, renewals</small></button>
      </div>
      {yearly ? <YearlyFeeCalculator disabled={saving} automatic onApply={setAmount} /> : null}
      <div className="category-preview"><span className="budget-illustration" aria-hidden="true">{yearly ? '◷' : '◧'}</span><div><span className="eyebrow">YOUR NEW CATEGORY</span><strong>{name.trim() || 'Category name'}</strong><small>{yearly ? 'Build up a balance for the next fee' : 'A recurring monthly allocation'}</small></div><div><strong>{monthlyBudgetAmount(amount) === null ? '—' : currency(monthlyBudgetAmount(amount))}</strong><small>per month</small></div></div>
      <p>Starts in {monthLabel(currentMonthKey())} with no carryover. Set €0 to fund it later.</p>
      {duplicate ? <p role="alert">A category with this name already exists.</p> : null}
      {!yearly && monthlyBudgetAmount(amount) === null ? <p role="alert">Enter a non-negative monthly amount.</p> : null}
      <button type="submit" className="button button-primary" disabled={!valid || saving || !!mutation}>{saving ? 'Creating…' : 'Create category'}</button>
    </form> : null}
    {message ? <p role="status">{message}</p> : null}
  </div>;
}

function MonthlyBudgetEditor({ snapshot, onRefresh, mutation, onMutation }: PageProps & {
  onRefresh: () => Promise<boolean>; mutation: string; onMutation: (value: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [amounts, setAmounts] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [effectiveMonth, setEffectiveMonth] = useState(currentMonthKey());
  const [message, setMessage] = useState('');
  const [scheduledName, setScheduledName] = useState<string | null>(null);
  const [feeCategory, setFeeCategory] = useState('');
  const [categorySearch, setCategorySearch] = useState('');
  useEffect(() => {
    setEditing(false);
    setAmounts(Object.fromEntries(snapshot.budgets.map(item => [item.name, String(item.monthlyEur ?? 0)])));
  }, [snapshot.budgetMonth, snapshot.budgets]);
  const allocationCategories = snapshot.budgets.filter(item => item.name !== "Unallocated");
  const valid = allocationCategories.length > 0 && allocationCategories.every(item => monthlyBudgetAmount(amounts[item.name] ?? '') !== null);
  const total = allocationCategories.reduce((sum, item) => {
    const amount = editing ? monthlyBudgetAmount(amounts[item.name] ?? '') : item.monthlyEur ?? 0;
    return amount === null ? Number.NaN : sum + Math.round(amount * 100);
  }, 0) / 100;
  const incomeEur = snapshot.incomeExpectation.amountEur;
  const difference = incomeEur === null ? null : Math.round((incomeEur - total) * 100) / 100;
  const current = snapshot.budgetMonth === currentMonthKey();
  const originalTotal = allocationCategories.reduce((sum, item) => sum + Math.round(numeric(item.monthlyEur) * 100), 0) / 100;
  const changedCount = allocationCategories.filter(item => monthlyBudgetAmount(amounts[item.name] ?? '') !== numeric(item.monthlyEur)).length;
  const visibleCategories = allocationCategories.filter(item => (!scheduledName || item.name === scheduledName) && item.name.toLocaleLowerCase().includes(categorySearch.trim().toLocaleLowerCase()));
  async function save() {
    if (!valid || saving || mutation || effectiveMonth < currentMonthKey()) return;
    const changedBudgets = allocationCategories.filter(item => {
      const amount = monthlyBudgetAmount(amounts[item.name] ?? '');
      return scheduledName ? item.name === scheduledName : effectiveMonth !== currentMonthKey() || amount !== (item.monthlyEur ?? 0);
    });
    if (!changedBudgets.length) { setEditing(false); return; }
    setSaving(true); setMessage(''); onMutation('Saving monthly budget…');
    try {
      await saveMonthlyBudget(effectiveMonth, changedBudgets.map(item => ({ name: item.name, amount: String(monthlyBudgetAmount(amounts[item.name] ?? '')!) })));
      if (await onRefresh()) { setEditing(false); setMessage('Monthly budget saved.'); }
      else { setEditing(false); setMessage('Monthly budget saved. Refresh to load the updated balances.'); }
    } catch (error) { setMessage(error instanceof Error ? error.message : 'Could not save monthly budget.'); }
    finally { setSaving(false); onMutation(''); }
  }
  return <Panel>
    <SectionHeading eyebrow="MONTHLY PLAN" title="Monthly budget" detail="Set how much each category receives every month."
      action={current ? <button type="button" className="text-button" disabled={saving || !!mutation} onClick={() => { setAmounts(Object.fromEntries(snapshot.budgets.map(item => [item.name, String(item.monthlyEur ?? 0)]))); setScheduledName(null); setCategorySearch(''); setEffectiveMonth(currentMonthKey()); setEditing(!editing); setMessage(''); }}>{editing ? 'Cancel' : 'Edit monthly budget'}</button> : undefined} />
    <div className="funding-grid">
      <div><span>Total monthly plan</span><strong>{Number.isFinite(total) ? currency(total) : '—'}</strong></div>
      <div><span>Expected monthly income</span><strong>{incomeEur === null ? 'Rate unavailable' : currency(incomeEur)}</strong></div>
      <div><span>{difference !== null && difference < 0 ? 'Over monthly income' : 'Income left after budget'}</span><strong className={difference !== null && difference < 0 ? 'value-negative' : 'value-positive'}>{difference !== null && Number.isFinite(difference) ? currency(Math.abs(difference)) : '—'}</strong></div>
    </div>
    {incomeEur !== null && incomeEur > 0 && Number.isFinite(total) ? <div className="plan-income-visual"><div className="plan-income-track" role="meter" aria-label="Expected income allocated" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.min(100, Math.round(total / incomeEur * 100))} aria-valuetext={currency(total) + ' allocated of ' + currency(incomeEur)}><span className={total > incomeEur ? 'over-income' : ''} style={{ width: Math.min(100, total / incomeEur * 100) + '%' }} /></div><div><span>{Math.round(total / incomeEur * 100)}% of expected income allocated</span><span>{total > incomeEur ? 'Over income' : currency(incomeEur - total) + ' unassigned'}</span></div></div> : null}
    {!editing ? <details className="monthly-plan-details"><summary>View allocations · {allocationCategories.length} categories</summary><div className="monthly-plan-list">{allocationCategories.map(item => <div key={item.name}><span>{item.name}</span><strong>{currency(item.monthlyEur)}<small> / month</small></strong></div>)}</div></details> : null}
    <p className="funding-detail">Expected income is {currency(snapshot.incomeExpectation.amount, snapshot.incomeExpectation.currency)} per month. {current ? 'Changes apply from ' + monthLabel(snapshot.budgetMonth) + ' onward, until the next scheduled change. Past months are preserved.' : 'Select the current month to edit the monthly plan.'}</p>
    {snapshot.scheduledAllocations?.length ? <div><strong>Scheduled changes</strong><p>Edit upcoming allocations, or cancel a future change to continue the previous plan. Changes already in effect can be replaced from the current month using Edit monthly budget.</p>{snapshot.scheduledAllocations.map(item => <p key={item.effectiveStartMonth + item.name}>{monthLabel(item.effectiveStartMonth)} · {item.name}: {currency(item.amountEur)}/month <button type="button" className="text-button" disabled={!!mutation || item.effectiveStartMonth < currentMonthKey()} onClick={() => { setScheduledName(item.name); setEffectiveMonth(item.effectiveStartMonth); setAmounts(previous => ({ ...previous, [item.name]: String(item.amountEur ?? 0) })); setEditing(true); }}>Edit</button> <button type="button" className="text-button" disabled={!!mutation || item.effectiveStartMonth <= currentMonthKey()} onClick={async () => {
      if (!window.confirm('Cancel the ' + item.name + ' change in ' + monthLabel(item.effectiveStartMonth) + '? The previous plan will continue until the next scheduled change.')) return;
      onMutation('schedule'); setMessage('');
      try { await cancelMonthlyBudget(item.effectiveStartMonth, item.name); const refreshed = await onRefresh(); setMessage('Scheduled change cancelled.' + (refreshed ? '' : ' Refresh to see the updated plan.')); }
      catch (error) { setMessage('Cancellation failed. ' + (error instanceof Error ? error.message : String(error))); }
      finally { onMutation(''); }
    }}>Cancel change</button></p>)}</div> : null}
    {editing ? <form className="monthly-plan-editor" onSubmit={event => { event.preventDefault(); void save(); }}>
      <label className="monthly-plan-date"><span>Apply this plan from</span><input type="month" disabled={!!scheduledName || saving} min={currentMonthKey()} value={effectiveMonth} onChange={event => setEffectiveMonth(event.target.value)} required /></label>
      <details className="yearly-fee-details">
        <summary>Plan for a yearly fee</summary>
        <label className="yearly-fee-category"><span>Category to save in</span><select value={scheduledName ?? feeCategory} disabled={saving || !!mutation || !!scheduledName} onChange={event => setFeeCategory(event.target.value)}><option value="">Choose a category</option>{allocationCategories.map(item => <option key={item.name} value={item.name}>{item.name}</option>)}</select></label>
        {(scheduledName ?? feeCategory) ? <YearlyFeeCalculator key={scheduledName ?? feeCategory} disabled={saving || !!mutation} startMonth={effectiveMonth} saved={effectiveMonth === currentMonthKey() ? numeric(allocationCategories.find(item => item.name === (scheduledName ?? feeCategory))?.remainingEur) - numeric(allocationCategories.find(item => item.name === (scheduledName ?? feeCategory))?.monthlyEur) : 0} onApply={value => setAmounts(previous => ({ ...previous, [scheduledName ?? feeCategory]: value }))} /> : null}
        {effectiveMonth !== currentMonthKey() ? <p className="funding-detail">Future balances are unknown; this estimate starts with €0 saved.</p> : null}
      </details>
      <div className="allocation-toolbar"><label><span>Find a category</span><input type="search" placeholder="Search categories" value={categorySearch} onChange={event => setCategorySearch(event.target.value)} /></label><span>{changedCount} {changedCount === 1 ? 'category changed' : 'categories changed'}</span></div>
      <div className="allocation-cards">{visibleCategories.map(item => {
        const next = monthlyBudgetAmount(amounts[item.name] ?? '');
        const previous = numeric(item.monthlyEur);
        const changed = next !== previous;
        const share = next !== null && Number.isFinite(total) && total > 0 ? next / total * 100 : 0;
        const delta = next === null ? null : Math.round((next - previous) * 100) / 100;
        return <div className={'allocation-card' + (changed ? ' allocation-card-changed' : '')} key={item.name}>
          <div className="allocation-card-heading"><strong><i aria-hidden="true" />{item.name}</strong>{changed ? <span className="allocation-change">{delta === null ? 'Check amount' : delta === 0 ? 'Unchanged' : (delta > 0 ? '+' : '−') + currency(Math.abs(delta))}</span> : <span className="allocation-share">{share.toFixed(1)}% of plan</span>}</div>
          <label className="allocation-amount"><span>EUR / month</span><NumericInput aria-label={item.name + ' monthly allocation'} value={amounts[item.name] ?? ''} disabled={saving || !!mutation} onChange={value => setAmounts(previous => ({ ...previous, [item.name]: value }))} /></label>
          <div className="allocation-share-track" aria-hidden="true"><span style={{ width: Math.min(100, share) + '%' }} /></div>
          <div className="allocation-card-foot"><span>{changed ? 'Was ' + currency(previous) + ' / month' : currency(next === null ? 0 : next * 12) + ' over a year'}</span><button type="button" className="text-button" disabled={saving || !!mutation} onClick={() => { setFeeCategory(item.name); const details = document.querySelector<HTMLDetailsElement>('.yearly-fee-details'); if (details) { details.open = true; details.scrollIntoView({ behavior: 'smooth', block: 'center' }); } }}>Yearly fee ↗</button></div>
        </div>;
      })}</div>
      {!visibleCategories.length ? <p>No categories match your search.</p> : null}
      {difference !== null && difference < 0 ? <p role="status" className="value-negative">The monthly plan exceeds expected income by {currency(-difference)}. You can still save this plan.</p> : null}
      {!valid ? <ul role="status">{allocationCategories.filter(item => monthlyBudgetAmount(amounts[item.name] ?? '') === null).map(item => <li key={item.name}>{item.name}: enter a non-negative amount with at most two decimal places.</li>)}</ul> : null}
      {effectiveMonth < currentMonthKey() || !/^\d{4}-(0[1-9]|1[0-2])$/.test(effectiveMonth) ? <p>Effective month: choose the current month or a future month.</p> : null}
      <div className="monthly-plan-actions"><span className="plan-change-summary">{Number.isFinite(total) ? (total === originalTotal ? 'Monthly total unchanged' : currency(originalTotal) + ' → ' + currency(total) + ' / month') : 'Check the highlighted amounts'}</span><button type="button" className="button button-secondary" disabled={saving} onClick={() => setEditing(false)}>Discard changes</button><button className="button button-primary" type="submit" disabled={!valid || saving || !!mutation || effectiveMonth < currentMonthKey() || !/^\d{4}-(0[1-9]|1[0-2])$/.test(effectiveMonth)}>{saving ? 'Saving…' : 'Save monthly budget'}</button></div>
    </form> : null}
    {!editing && current ? <NewBudgetCategory snapshot={snapshot} onRefresh={onRefresh} mutation={mutation} onMutation={onMutation} /> : null}
    {message ? <p role="status">{message}</p> : null}
  </Panel>;
}

function BudgetsPage({ snapshot, ignoreReallocation, onSelectMonth, onToggleReallocation, onRefresh, onMutation, mutation, onNavigate, onDrillDown }: PageProps & {
  ignoreReallocation: boolean;
  onSelectMonth: (month: string) => void;
  onToggleReallocation: () => void;
  onRefresh: () => Promise<boolean>;
  onMutation: (value: string) => void;
  mutation: string;
  onNavigate: (page: PageKey, month?: string) => void;
}) {
  const budgets = snapshot.budgets;
  const donors = budgets.filter((item) => budgetBalance(item.remainingEur) > 0).sort((a, b) => numeric(b.remainingEur) - numeric(a.remainingEur));
  const deficits = budgets.filter((item) => budgetBalance(item.remainingEur) < 0).sort((a, b) => numeric(a.remainingEur) - numeric(b.remainingEur));
  const toBudgetOptions = (items: typeof budgets) => items.map((item): BudgetSelectOption => {
    const detail = currency(item.remainingEur) + ' left';
    return {
      value: item.name,
      label: item.name + ' · ' + detail,
      categoryName: item.name,
      detail,
      searchText: item.name + ' ' + detail,
    };
  });
  const sourceOptions = toBudgetOptions([...budgets].sort((a, b) => numeric(b.remainingEur) - numeric(a.remainingEur)));
  const destinationOptions = toBudgetOptions([...budgets].sort((a, b) => numeric(a.remainingEur) - numeric(b.remainingEur)));
  const [source, setSource] = useState('');
  const [destination, setDestination] = useState('');
  const [moveAmount, setMoveAmount] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [localError, setLocalError] = useState('');
  const [pendingTransactionId, setPendingTransactionId] = useState<string | null>(null);

  useEffect(() => {
    setSource((current) => budgets.some((item) => item.name === current) ? current : donors[0]?.name ?? budgets[0]?.name ?? '');
    setDestination((current) => budgets.some((item) => item.name === current) ? current : deficits[0]?.name ?? budgets.find((item) => item.name !== (donors[0]?.name ?? budgets[0]?.name))?.name ?? '');
  }, [snapshot.budgetMonth, snapshot.budgets]);

  const sourceBudget = budgets.find((item) => item.name === source);
  const destinationBudget = budgets.find((item) => item.name === destination);
  const suggested = sourceBudget && destinationBudget
    ? Math.max(0, Math.min(budgetBalance(sourceBudget.remainingEur), -budgetBalance(destinationBudget.remainingEur)))
    : 0;

  const previewAmount = moveAmount.trim() ? parseAmount(moveAmount) : suggested;
  const proposal = balancingProposal(budgets);
  const moveValid = !!sourceBudget && !!destinationBudget && source !== destination && previewAmount !== null && previewAmount > 0;
  async function balanceBudget() {
    setLocalError('');
    const amount = moveAmount.trim() ? parseAmount(moveAmount) : suggested;
    if (!sourceBudget || !destinationBudget) {
      setLocalError('Choose a source and destination category.');
      return;
    }
    if (source === destination || amount === null || !Number.isFinite(amount) || amount <= 0) {
      setLocalError('The move must be greater than zero and use two different categories.');
      return;
    }

    setSubmitting(true);
    onMutation('reallocate');
    try {
      const transactionId = pendingTransactionId ?? uuidV7();
      setPendingTransactionId(transactionId);
      await submitTransaction({
        transactionId,
        kind: 'reallocate',
        dateOnly: true,
        date: lastDayOfMonth(snapshot.budgetMonth),
        description: 'Month-end budget balancing',
        amount,
        fromCategory: source,
        toCategory: destination,
        fromAccount: 'Reallocation',
        toAccount: 'Reallocation',
      });
      setMoveAmount('');
      setPendingTransactionId(null);
      const refreshed = await onRefresh();
      setLocalError('Budget move saved. ' + source + ': ' + currency(numeric(sourceBudget.remainingEur) - amount) + ' remaining; ' + destination + ': ' + currency(numeric(destinationBudget.remainingEur) + amount) + ' remaining.' + (refreshed ? '' : ' Refresh failed; these balances reflect this move only. Refresh before making another move.'));
    } catch (error) {
      setLocalError(error instanceof Error ? error.message : String(error));
    } finally {
      setSubmitting(false);
      onMutation('');
    }
  }

  const timeline: BudgetTimelineDatum[] = [...(snapshot.budgetHistory ?? [])]
    .sort((left, right) => left.month.localeCompare(right.month))
    .map((item) => ({
      month: item.month,
      categories: item.categories.map((category) => ({
        name: category.name,
        amount: Math.max(0, budgetUse(category, ignoreReallocation)),
      })),
    }));

  return (
    <>
      <div className="page-intro">
        <div><div className="eyebrow">BUDGET CONTROL</div><h1>Balance the month</h1><p>Review remaining funds, then move surplus into categories below zero. Any open month stays available for balancing until you close it.</p></div>
        <label className="check-switch"><input type="checkbox" checked={ignoreReallocation} onChange={onToggleReallocation} /><span>Adjust spending for budget moves</span></label>
      </div>
      <div className="budget-summary-grid">
        <div className="summary-tile"><span>Total remaining</span><strong>{currency(budgets.reduce((total, item) => total + numeric(item.remainingEur), 0))}</strong></div>
        <div className="summary-tile"><span>Categories below zero</span><strong className={deficits.length ? 'value-negative' : ''}>{deficits.length}</strong></div>
        <div className="summary-tile"><span>Positive categories</span><strong>{donors.length}</strong></div>
      </div>
      <MonthlyBudgetEditor snapshot={snapshot} onRefresh={onRefresh} onMutation={onMutation} mutation={mutation} />
      <Panel>
        <SectionHeading eyebrow="SPENDING HISTORY" title="Spending by category over time" detail="Stacked EUR spending across every recorded month. Hover for a category breakdown or select a month to open it." />
        {timeline.length ? <BudgetSpendingTimeline
          data={timeline}
          activeMonth={snapshot.budgetMonth}
          onSelectMonth={onSelectMonth}
          formatMonth={monthLabel}
          formatCurrency={currency}
          formatCompactCurrency={compactCurrency}
        /> : <EmptyState title="No spending history yet" detail="Spending by category will appear here as activity is recorded." />}
      </Panel>
      <BudgetMonthLifecycle onNavigate={onNavigate} snapshot={snapshot} onRefresh={onRefresh} onMutation={onMutation} mutation={mutation} />
      <div className="budget-composition-grid">
        <Panel>
          <SectionHeading eyebrow="ALLOCATION MIX" title="Monthly allocation shares" detail={monthLabel(snapshot.budgetMonth) + ' · Each category’s share of the monthly plan.'} />
          <BudgetComposition data={budgets.filter((item) => item.name !== 'Unallocated').map((item) => ({ name: item.name, amount: item.monthlyEur }))} categoryNames={budgets.map((item) => item.name).sort((a, b) => a.localeCompare(b))} formatCurrency={currency} />
        </Panel>
        <Panel>
          <SectionHeading eyebrow="REMAINING MIX" title="Remaining budget shares" detail={monthLabel(snapshot.budgetMonth) + ' · Accumulated category balances, including carryover and Unallocated.'} />
          <BudgetComposition data={budgets.map((item) => ({ name: item.name, amount: item.remainingEur }))} categoryNames={budgets.map((item) => item.name).sort((a, b) => a.localeCompare(b))} formatCurrency={currency} remaining />
        </Panel>
      </div>
      <Panel>
        <SectionHeading eyebrow="MONTH-END ALLOCATION" title="Move budget between categories" detail={'Recorded as a budget reallocation dated ' + dateLabel(lastDayOfMonth(snapshot.budgetMonth)) + '.'} />
        {budgets.length >= 2 && snapshot.canReallocate ? (
          <> {moveValid ? <p role="status">{source}: {currency(sourceBudget?.remainingEur)} → {currency(numeric(sourceBudget?.remainingEur) - previewAmount!)} · {destination}: {currency(destinationBudget?.remainingEur)} → {currency(numeric(destinationBudget?.remainingEur) + previewAmount!)}{numeric(sourceBudget?.remainingEur) - previewAmount! < 0 ? ' · Warning: this creates or increases a source deficit.' : ''}</p> : <p>Choose different categories and enter an amount greater than zero.</p>}
          {proposal.moves.length ? <details><summary>Review a balancing proposal using available categories</summary><p>Review each move before recording it. The proposal updates after each saved move.</p>{proposal.moves.map((move, index) => <p key={index}>{move.from} → {move.to}: {currency(move.amount)} <button type="button" className="text-button" disabled={!!mutation} onClick={() => { setSource(move.from); setDestination(move.to); setMoveAmount(String(move.amount)); setPendingTransactionId(null); }}>Preview this move</button></p>)}{proposal.uncovered > 0 ? <p>Available funds leave {currency(proposal.uncovered)} uncovered.</p> : null}</details> : null}
          <div className="move-form">
            <label><span>Move from</span><Select<string, BudgetSelectOption>
              id="budget-move-source"
              aria-label="Move from"
              className="budget-category-select"
              classNames={{ popup: { root: 'budget-category-dropdown' } }}
              showSearch={{ filterOption: (input, option) => (option?.searchText ?? '').toLocaleLowerCase().includes(input.toLocaleLowerCase()) }}
              optionRender={(option) => <div className="budget-select-option"><span>{option.data.categoryName}</span><small>{option.data.detail}</small></div>}
              options={sourceOptions}
              value={source || undefined}
              placeholder="Choose source"
              listHeight={240}
              onChange={(value) => { setSource(value); setMoveAmount(''); setPendingTransactionId(null); }}
            /></label>
            <span className="move-arrow">→</span>
            <label><span>Move to</span><Select<string, BudgetSelectOption>
              id="budget-move-destination"
              aria-label="Move to"
              className="budget-category-select"
              classNames={{ popup: { root: 'budget-category-dropdown' } }}
              showSearch={{ filterOption: (input, option) => (option?.searchText ?? '').toLocaleLowerCase().includes(input.toLocaleLowerCase()) }}
              optionRender={(option) => <div className="budget-select-option"><span>{option.data.categoryName}</span><small>{option.data.detail}</small></div>}
              options={destinationOptions}
              value={destination || undefined}
              placeholder="Choose destination"
              listHeight={240}
              onChange={(value) => { setDestination(value); setMoveAmount(''); setPendingTransactionId(null); }}
            /></label>
            <label className="amount-field"><span>Amount in EUR</span><NumericInput value={moveAmount} onChange={value => { setMoveAmount(value); setPendingTransactionId(null); }} placeholder={currency(suggested).replace('€', '').trim()} /></label>
            <div className="move-submit"><button className="button button-primary" type="button" disabled={!moveValid || submitting || mutation !== ''} onClick={balanceBudget}>{submitting ? 'Recording…' : 'Record budget move'}</button>{suggested > 0 ? <small>Suggested move: {currency(suggested)}</small> : null}</div>
          </div></>
        ) : budgets.length < 2 ? (
          <EmptyState title="No categories to move between" detail="At least two budget categories are needed to record a move." />
        ) : (
          <EmptyState title="Rebalancing is unavailable" detail="Select an open budget month to move money between categories. Closed balances are fixed." />
        )}
        {localError ? <p className="inline-error" role="alert">{localError}</p> : null}
      </Panel>
      <Panel>
        <SectionHeading eyebrow="BUDGET DETAIL" title="Category balances" detail="Monthly plan, recorded spending and accumulated available budget." />
        <CategoryTable snapshot={snapshot} ignoreReallocation={ignoreReallocation} onDrillDown={onDrillDown} />
      </Panel>
    </>
  );
}

function SavedReconciliationPanel({ batch, onStartAnother }: {
  batch: ReconciliationBatch;
  onStartAnother: () => void;
}) {
  const month = batch.effectiveDate.slice(0, 7);
  const submittedAt = new Date(batch.submittedAt);
  const submittedLabel = Number.isNaN(submittedAt.valueOf())
    ? dateLabel(batch.submittedAt)
    : submittedAt.toLocaleString('en-GB', { dateStyle: 'medium', timeStyle: 'short' });

  return <Panel className={'saved-reconciliation ' + (batch.isComplete ? 'saved-reconciliation-complete' : '')}>
    <div className="saved-reconciliation-heading">
      <div className="saved-reconciliation-mark">{batch.isComplete ? '✓' : '↔'}</div>
      <div><div className="eyebrow">{monthLabel(month).toUpperCase()} · SAVED RECONCILIATION</div>
        <h2>{batch.isComplete ? monthLabel(month) + ' reconciliation finished' : monthLabel(month) + ' reconciliation is partial'}</h2>
        <p>{batch.isCurrent === false ? 'Account activity changed after this run. Reconcile again before closing the budget month.' : batch.isComplete
          ? 'This month was successfully reconciled. The saved account balances and adjustments are shown below.'
          : 'This saved run does not include every active account yet. Finish the reconciliation below.'}</p>
      </div>
      <Tag tone={batch.isCurrent === false || !batch.isComplete ? 'warn' : 'good'}>{batch.isCurrent === false ? 'Refresh needed' : batch.isComplete ? 'Finished' : 'Partial'}</Tag>
    </div>
    <div className="saved-reconciliation-meta">
      <div><span>Accounts reconciled</span><strong>{batch.reconciledAccountCount} / {batch.totalAccountCount}</strong></div>
      <div><span>Submitted</span><strong>{submittedLabel}</strong></div>
      <div><span>Balances observed</span><strong>{batch.observedDate ? dateLabel(batch.observedDate) : '—'}</strong></div>
    </div>
    <Table<ReconciliationBatch['accounts'][number]>
      className="sortable-table saved-reconciliation-table"
      rowKey="id"
      dataSource={batch.accounts}
      pagination={false}
      scroll={{ x: 'max-content' }}
      columns={[
        { title: 'Account', key: 'account', sorter: (a, b) => a.name.localeCompare(b.name), render: (_, account) => <><strong>{account.name}</strong><small>{account.accountType || 'Account'} · {account.currency ?? 'EUR'}</small></> },
        { title: 'Observed balance', key: 'observed', align: 'right', sorter: (a, b) => compareNullableNumbers(a.observedBalanceNative, b.observedBalanceNative), render: (_, account) => account.observedBalanceNative === null ? '—' : currency(account.observedBalanceNative, account.currency ?? 'EUR') },
        { title: 'Closing balance', key: 'closing', align: 'right', sorter: (a, b) => compareNullableNumbers(a.closingBalanceNative, b.closingBalanceNative), render: (_, account) => account.closingBalanceNative === null ? '—' : currency(account.closingBalanceNative, account.currency ?? 'EUR') },
        { title: 'Adjustment', key: 'adjustment', align: 'right', sorter: (a, b) => compareNullableNumbers(a.varianceNative, b.varianceNative), render: (_, account) => account.varianceNative === null ? '—' : signedCurrency(account.varianceNative, account.currency ?? 'EUR') },
        { title: 'Activity after close', key: 'activity', align: 'right', sorter: (a, b) => compareNullableNumbers(a.postMonthActivityNative, b.postMonthActivityNative), render: (_, account) => account.postMonthActivityNative === null ? '—' : signedCurrency(account.postMonthActivityNative, account.currency ?? 'EUR') },
      ]}
    />
    {batch.isComplete ? <div className="saved-reconciliation-action"><span>Need to replace the saved balances?</span><button className="text-button" type="button" onClick={onStartAnother}>Start another reconciliation ↗</button></div> : null}
  </Panel>;
}

function ReconciliationNote({ storageKey }: { storageKey: string }) {
  const [note, setNote] = useState(() => { try { return localStorage.getItem(storageKey) ?? ''; } catch { return ''; } });
  const [failed, setFailed] = useState(false);
  return <label>Investigation notes (on this device)<textarea value={note} onChange={event => { setNote(event.target.value); try { localStorage.setItem(storageKey, event.target.value); setFailed(false); } catch { setFailed(true); } }} />{failed ? <small role="alert">Notes could not be stored. Keep a copy before leaving.</small> : null}</label>;
}

function TransactionHistory({ id, revision }: { id: string; revision: string }) {
  const [history, setHistory] = useState<Awaited<ReturnType<typeof getTransactionHistory>>['history'] | null>(null);
  const [error, setError] = useState('');
  const load = async () => { setError(''); try { setHistory((await getTransactionHistory(id)).history); } catch { setError('Could not load edit history. Retry below.'); } };
  return <details className="transaction-history" onToggle={event => { if (event.currentTarget.open && !history) void load(); }}>
    <summary>Edit history · version {revision}</summary>
    {error ? <p className="inline-error" role="alert">{error} <button className="text-button" type="button" onClick={() => void load()}>Retry</button></p> : history ? history.length ?
      <ol className="history-timeline">{[...history].sort((a, b) => b.revision - a.revision).map(entry => {
        const value = entry.details.request ?? entry.details.transaction;
        const accounts = [value?.fromAccountName, value?.toAccountName].filter(Boolean).join(' → ');
        const categories = [value?.fromCategoryName, value?.toCategoryName].filter(Boolean).join(' → ');
        return <li key={entry.revision}>
          <div className="history-version">Version {entry.revision} · {entry.by ?? 'household / import'}</div>
          <time dateTime={entry.at}>{new Date(entry.at).toLocaleString()}</time>
          {value?.description ? <p className="history-description">{value.description}</p> : null}
          <div className="history-facts">
            {value?.amount ? <span className="history-amount">{value.amount} {value.inputCurrency}</span> : null}
            {value?.date ? <span>{dateLabel(value.date)}</span> : null}
            {accounts ? <span>Accounts · {accounts}</span> : null}
            {categories ? <span>Categories · {categories}</span> : null}
          </div>
        </li>;
      })}</ol> : <p className="history-empty">No recorded changes for this imported entry.</p> : <p className="history-empty" role="status">Loading history…</p>}
  </details>;
}

function ReconciliationPage({ onDrillDown, user, preview, actualBalances, setActualBalances, mutation, loading, onSubmit, onRefresh }: {
  onDrillDown: (filter: LedgerFilter) => void; user: string;
  preview: ReconciliationPreview;
  actualBalances: Record<string, string>;
  setActualBalances: (updater: (current: Record<string, string>) => Record<string, string>) => void;
  mutation: string; loading: boolean; onSubmit: () => void; onRefresh: () => void;
}) {
  const [recordAnother, setRecordAnother] = useState(false);
  const saved = preview.reconciliation;
  const showEntryForm = !saved?.isComplete || saved?.isCurrent === false || recordAnother || Object.keys(actualBalances).length > 0;
  const filled = preview.accounts.length > 0 && preview.accounts.every((account) => parseAmount(actualBalances[account.name] ?? '') !== null);
  const month = preview.closingDate.slice(0, 7);
  const observationLabel = new Date(preview.observedAt).toLocaleString('en-GB', { dateStyle: 'medium', timeStyle: 'short' });
  const convert = (value: number, code: Currency, closing = false) => {
    const rate = preview.rates.find((item) => item.currency === code)!;
    return Math.round(value / (closing ? rate.closingUnitsPerEur : rate.unitsPerEur) * 100) / 100;
  };
  const statementTotal = filled ? preview.accounts.reduce((total, account) => total + convert(parseAmount(actualBalances[account.name])!, account.currency), 0) : null;
  const closingStatementTotal = filled ? preview.accounts.reduce((total, account) => total + convert(parseAmount(actualBalances[account.name])! - account.activityNative, account.currency, true), 0) : null;
  const closingDeltaEur = closingStatementTotal === null ? null : preview.closingTotalEur - closingStatementTotal;

  return <>
    <div className="page-intro"><div><div className="eyebrow">ACCOUNT CLOSE · {monthLabel(month).toUpperCase()}</div><h1>Reconcile your accounts</h1><p>{saved?.isComplete
      ? monthLabel(month) + ' has already been successfully reconciled. Review the saved account results below.'
      : 'Enter your current bank and wallet balances. We reconstruct the closing balances for ' + dateLabel(preview.closingDate) + '.'}</p></div></div>
    {saved ? <SavedReconciliationPanel batch={saved} onStartAnother={() => { setActualBalances(() => ({})); setRecordAnother(true); }} /> : null}
    {showEntryForm ? <>
    <Panel className="recon-note"><span className="recon-note-icon">i</span><p>Recorded balances checked at <strong>{observationLabel}</strong>. Recorded activity after {dateLabel(preview.closingDate)} is deducted from your current balances. Record any missing transactions before posting. Future-dated transactions are excluded; your current balances are captured when you post.</p></Panel>
    <Panel>
      <SectionHeading eyebrow="ACCOUNTS IN EUR" title="Current account totals" action={<button className="text-button" disabled={loading || mutation !== ''} type="button" onClick={onRefresh}>{loading ? 'Refreshing…' : 'Refresh balance check'}</button>} />
      <div className="funding-grid">
        <div><span>Recorded total</span><strong>{currency(preview.totalEur)}</strong></div>
        <div><span>Current statement total</span><strong>{statementTotal === null ? 'Enter all balances' : currency(statementTotal)}</strong></div>
        <div><span>Reconstructed closing statement total</span><strong>{closingStatementTotal === null ? '—' : currency(closingStatementTotal)}</strong></div>
        <div><span>Total reconciliation delta</span><strong className={closingDeltaEur === null ? '' : closingDeltaEur > 0.01 ? 'value-negative' : closingDeltaEur < -0.01 ? 'value-positive' : ''}>{closingDeltaEur === null ? '—' : signedCurrency(closingDeltaEur, 'EUR')}</strong></div>
      </div>
      <p className="funding-detail">Current totals use the latest available rates up to {dateLabel(preview.observedDate)}. Closing totals use the last available rates up to {dateLabel(preview.closingDate)}.</p>
      <p className="funding-detail">{preview.rates.filter(rate => rate.currency !== 'EUR').map(rate => <span key={rate.currency}>{rate.currency}: current rate {dateLabel(rate.date)}, closing rate {dateLabel(rate.closingRateDate)}. </span>)}</p>
    </Panel>
    <Panel>
      <SectionHeading eyebrow="CURRENT BALANCES" title="Statement against recorded balance" detail="Enter each account’s balance now in its native currency. The closing adjustment replaces any previous adjustment for that month; a positive amount reduces the balance." />
      <div className="reconciliation-list">
        {preview.accounts.map((account) => {
          const actual = parseAmount(actualBalances[account.name] ?? '');
          const closingActual = actual === null ? null : actual - account.activityNative;
          const variance = actual === null ? null : account.balanceNative + account.existingAdjustmentNative - actual;
          return <div className="reconciliation-account" key={account.id}>
            <div className="reconciliation-row">
              <div className="recon-account"><span className="account-symbol">⌂</span><div><strong>{account.name}</strong><small>{account.type || 'Account'} · {account.currency}</small></div></div>
              <div className="balance-cell"><small>Recorded balance</small><strong>{currency(account.balanceNative, account.currency)}</strong></div>
              <label className="balance-entry"><small>Current statement balance</small><span className="money-input"><NumericInput disabled={mutation !== ''} value={actualBalances[account.name] ?? ''} onChange={value => setActualBalances(current => ({ ...current, [account.name]: value }))} placeholder="0.00" aria-label={'Current balance for ' + account.name} /><em>{account.currency}</em></span></label>
              <div className="variance-cell"><small>Closing adjustment</small><strong className={variance === null ? '' : variance > 0.01 ? 'value-negative' : variance < -0.01 ? 'value-positive' : ''}>{variance === null ? '—' : signedCurrency(variance, account.currency)}</strong></div>
            </div>
            {variance !== null && Math.abs(variance) > 0.005 ? <div className="discrepancy-review"><button className="text-button" type="button" onClick={() => onDrillDown({ account: account.name })}>Inspect {account.name} transactions ↗</button><p>Compare your statement with recorded purchases. Check missing expenses, duplicate receipts, both sides of transfers, and bank fees before accepting an adjustment.</p><ReconciliationNote storageKey={'presto-recon-note:' + user + ':' + preview.closingDate + ':' + account.id} /></div> : null}
            <div className="reconciliation-bridge"><span>Activity since month-end: <strong>{signedCurrency(account.activityNative, account.currency)}</strong></span><span>Reconstructed closing balance: <strong>{closingActual === null ? '—' : currency(closingActual, account.currency)}</strong></span>
              {account.transactions.length ? <details><summary>{account.transactions.length} recorded transaction{account.transactions.length === 1 ? '' : 's'} included</summary><ul>{account.transactions.map(item => <li key={item.id}><span>{transactionDateLabel(item)} · {item.description}</span><strong>{signedCurrency(item.netNative, account.currency)}</strong></li>)}</ul></details> : null}
            </div>
          </div>;
        })}
        {!preview.accounts.length ? <EmptyState title="No active accounts" detail="There are no active accounts to reconcile." /> : null}
      </div>
      <div className="recon-footer"><div>{!filled ? <p>Enter a valid statement balance for every account before reconciling.</p> : loading ? <p>Wait for the balance check to finish updating.</p> : null}<strong>Reconcile {monthLabel(month)} using today’s balances</strong><small>Current balances, reconstructed closing balances, and included activity are saved together. Changes to the accounts require a fresh balance check.</small></div><button className="button button-primary" type="button" disabled={!filled || loading || mutation !== ''} onClick={onSubmit}>{mutation === 'reconcile' ? 'Reconciling…' : 'Reconcile ' + monthLabel(month)}</button></div>
    </Panel>
    </> : null}
  </>;
}


function transactionDraft(snapshot: FinanceSnapshot, item?: GatewayTransaction): TransactionDraft {
  const accounts = snapshot.accounts;
  const categories = snapshot.budgets;
  const sourceAccount = item?.fromAccount && accounts.some(account => account.name === item.fromAccount)
    ? item.fromAccount : accounts[0]?.name ?? '';
  const destinationAccount = item?.toAccount && accounts.some(account => account.name === item.toAccount)
    ? item.toAccount : accounts.find(account => account.name !== sourceAccount)?.name ?? accounts[0]?.name ?? '';
  const category = (name: string | null | undefined) => name && categories.some(item => item.name === name)
    ? name : categories[0]?.name ?? '';
  const kind = item?.kind && ['expense', 'income', 'transfer', 'reallocate'].includes(item.kind)
    ? item.kind as SubmitTransaction['kind'] : 'expense';
  const defaultCurrency = kind === 'income'
    ? accounts.find(account => account.name === destinationAccount)?.currency
    : accounts.find(account => account.name === sourceAccount)?.currency;
  const today = new Date();
  return {
    kind,
    operationId: item?.transactionId ?? uuidV7(),
    baseRevision: item?.revision ?? '',
    incomeTreatment: item?.incomeTreatment ?? (item?.kind === 'income' ? 'legacy' : 'salary'),
    expectedIncomeMonth: item?.expectedIncomeMonth ?? snapshot.budgetMonth,
    correctsTransactionId: item?.correctsTransactionId ?? "",
    date: item?.date ?? (snapshot.budgetMonth === currentMonthKey() ? madridDateKey() : lastDayOfMonth(snapshot.budgetMonth)),
    description: item?.description ?? '',
    amount: item?.amount === null || item?.amount === undefined ? '' : String(Math.abs(item.amount)),
    currency: kind === 'reallocate' ? 'EUR' : item?.currency ?? defaultCurrency ?? 'EUR',
    toAmount: item?.toAmount === null || item?.toAmount === undefined ? '' : String(item.toAmount),
    fromCategory: category(kind === 'reallocate' ? item?.fromCategory : kind === 'expense' ? item?.fromCategory : undefined),
    toCategory: category(kind === 'income' || kind === 'reallocate' ? item?.toCategory : undefined),
    fromAccount: sourceAccount,
    toAccount: destinationAccount,
  };
}

function TransactionEditor({ snapshot, initial, saving, onCancel, onSave, user }: PageProps & {
  initial?: GatewayTransaction;
  user: string;
  saving: boolean;
  onCancel: () => void;
  onSave: (payload: SubmitTransaction, addAnother?: boolean) => Promise<TransactionDraft | null>;
}) {
  const draftKey = transactionDraftKey(user, snapshot.budgetMonth, initial?.transactionId ?? 'new');
  const [draft, setDraft] = useState(() => readTransactionDraft(draftKey, transactionDraft(snapshot, initial)));
  const [error, setError] = useState('');
  const [draftSaved, setDraftSaved] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(Boolean(draft.correctsTransactionId));
  useEffect(() => {
    const restored = readTransactionDraft(draftKey, transactionDraft(snapshot, initial));
    setDraft(restored); setError(''); setDirty(false); setDraftSaved(false); setDetailsOpen(Boolean(restored.correctsTransactionId));
  }, [draftKey]);
  useEffect(() => {
    if (dirty) setDraftSaved(writeTransactionDraft(draftKey, draft));
  }, [draftKey, draft, dirty]);
  useEffect(() => {
    const warn = (event: BeforeUnloadEvent) => {
      if (dirty && !draftSaved) { event.preventDefault(); event.returnValue = ''; }
    };
    const guard = (event: Event) => {
      if (saving) { event.preventDefault(); return; }
      if (dirty && !writeTransactionDraft(draftKey, draft) && !window.confirm('Your draft could not be saved on this device. Leave and lose these edits?')) event.preventDefault();
    };
    window.addEventListener('beforeunload', warn);
    window.addEventListener('presto-before-navigation', guard);
    return () => { window.removeEventListener('beforeunload', warn); window.removeEventListener('presto-before-navigation', guard); };
  }, [dirty, draftSaved, draftKey, draft, saving]);
  const discard = () => {
    if (!window.confirm('Discard this transaction draft? Your unsaved edits will be removed.')) return;
    if (!writeTransactionDraft(draftKey, null)) { setError('Could not remove the saved draft. Check browser storage and try again.'); return; }
    onCancel();
  };
  const sourceAccount = snapshot.accounts.find(account => account.name === draft.fromAccount);
  const destinationAccount = snapshot.accounts.find(account => account.name === draft.toAccount);
  const crossCurrency = draft.kind === 'transfer' && !!sourceAccount && !!destinationAccount && sourceAccount.currency !== destinationAccount.currency;
  const salaryExpectation = snapshot.budgetCoverage?.expectations.find(e => e.month === draft.expectedIncomeMonth);
  const amount = parseAmount(draft.amount);
  const toAmount = parseAmount(draft.toAmount);
  const dateInMonth = draft.date.startsWith(snapshot.budgetMonth + '-') && /^\d{4}-\d{2}-\d{2}$/.test(draft.date);
  const hasAccounts = draft.kind === 'expense' ? !!sourceAccount
    : draft.kind === 'income' ? !!destinationAccount
      : draft.kind === 'transfer' ? !!sourceAccount && !!destinationAccount && sourceAccount.name !== destinationAccount.name : true;
  const hasCategories = draft.kind === 'expense' ? !!draft.fromCategory
    : draft.kind === 'income' ? (draft.incomeTreatment === "salary" || draft.incomeTreatment === "unexpected" || !!draft.toCategory)
      : draft.kind === 'reallocate' ? !!draft.fromCategory && !!draft.toCategory && draft.fromCategory !== draft.toCategory : true;
  const valid = dateInMonth && !!draft.description.trim() && draft.description.length <= 500
    && amount !== null && amount > 0 && hasAccounts && hasCategories
    && (!crossCurrency || (toAmount !== null && toAmount > 0));
  function update<K extends keyof TransactionDraft>(key: K, value: TransactionDraft[K]) {
    setDirty(true);
    setDraft(current => ({ ...current, [key]: value }));
    setError('');
  }
  function changeKind(kind: SubmitTransaction['kind']) {
    setDirty(true);
    setDraft(current => ({
      ...current,
      kind,
      currency: kind === 'reallocate' ? 'EUR' : snapshot.accounts.find(account =>
        account.name === (kind === 'income' ? current.toAccount : current.fromAccount))?.currency ?? current.currency,
      fromCategory: kind === 'expense' || kind === 'reallocate' ? current.fromCategory || snapshot.budgets[0]?.name || '' : '',
      toCategory: kind === 'income' || kind === 'reallocate' ? current.toCategory || snapshot.budgets.find(item => item.name !== current.fromCategory)?.name || '' : '',
    }));
    setError('');
  }
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (saving) return;
    if (!valid || amount === null) {
      setError('Complete the required fields with a valid amount and a date in the selected month.');
      return;
    }
    if (draft.kind === 'transfer' && sourceAccount && destinationAccount && sourceAccount.name === destinationAccount.name) {
      setError('Choose two different accounts for a transfer.');
      return;
    }
    if (initial && draft.baseRevision !== initial.revision) {
      setError('This entry changed since your draft was saved. Discard the draft and review the latest transaction before editing.');
      return;
    }
    const addAnother = (event.nativeEvent as SubmitEvent).submitter?.getAttribute('data-add-another') === 'true';
    void onSave({
      transactionId: initial?.transactionId ?? draft.operationId,
      correctsTransactionId: draft.correctsTransactionId || undefined,
      kind: draft.kind,
      date: draft.date,
      occurredAt: initial?.date === draft.date ? initial.occurredAt : undefined,
      dateOnly: initial?.date === draft.date ? !initial.occurredAt : draft.kind === 'reallocate' || draft.date !== madridDateKey(),
      description: draft.description.trim(),
      amount,
      currency: draft.kind === 'reallocate' ? undefined : draft.currency,
      toAmount: crossCurrency && toAmount !== null ? toAmount : undefined,
      fromCategory: draft.kind === 'expense' || draft.kind === 'reallocate' ? draft.fromCategory : undefined,
      toCategory: draft.kind === 'income' ? (draft.incomeTreatment === 'refund' || draft.incomeTreatment === 'legacy' ? draft.toCategory : 'Unallocated') : draft.kind === 'reallocate' ? draft.toCategory : undefined,
      incomeTreatment: draft.kind === 'income' && draft.incomeTreatment !== 'legacy' ? draft.incomeTreatment : undefined,
      expectedIncomeMonth: draft.kind === 'income' && draft.incomeTreatment === 'salary' ? draft.expectedIncomeMonth : undefined,
      fromAccount: draft.kind === 'expense' || draft.kind === 'transfer' ? draft.fromAccount : undefined,
      toAccount: draft.kind === 'income' || draft.kind === 'transfer' ? draft.toAccount : undefined,
    }, addAnother).then(nextDraft => {
      if (nextDraft) { setDraft(nextDraft); setDirty(true); setDraftSaved(false); setDetailsOpen(false); setError(''); }
    });
  }

  return <Panel className="transaction-editor-panel">
    <SectionHeading eyebrow={initial ? 'EDIT TRANSACTION' : 'NEW TRANSACTION'} title={initial ? 'Edit transaction' : 'Add transaction'} detail="Transactions can be changed while their budget month remains open." />
    <form className="transaction-editor" onSubmit={submit}>
      <label><span>Type</span><select aria-label="Type" value={draft.kind} disabled={saving} onChange={event => changeKind(event.target.value as SubmitTransaction['kind'])}><option value="expense">Expense</option><option value="income">Income</option><option value="transfer">Transfer</option><option value="reallocate">Budget move</option></select></label>
      {draft.kind === 'income' ? <>
        <label><span>Income treatment</span><select aria-label="Income treatment" value={draft.incomeTreatment} disabled={saving} onChange={event => update('incomeTreatment', event.target.value as TransactionDraft['incomeTreatment'])}>
          <option value="salary">Salary · settle expected income</option><option value="unexpected">Unexpected income · Unallocated</option><option value="refund">Refund · restore category</option>
          {initial?.incomeTreatment === 'legacy' ? <option value="legacy">Preserve existing category credit</option> : null}
        </select></label>
        {draft.incomeTreatment === 'salary' ? <label><span>Expected salary month</span><select aria-label="Expected salary month" value={draft.expectedIncomeMonth} disabled={saving} onChange={event => update('expectedIncomeMonth', event.target.value)}>
          {snapshot.budgetCoverage?.expectations.filter(e => !e.completedMonth).map(e => <option key={e.month} value={e.month}>{monthLabel(e.month)} · {currency(e.outstandingEur)} outstanding</option>)}
        </select></label> : null}
        <p className="funding-detail">{draft.incomeTreatment === 'salary' ? 'Receipt increases cash. Your category allocations are already in the plan; any excess goes to Unallocated.' : draft.incomeTreatment === 'unexpected' ? 'Receipt increases cash and Unallocated. Allocate it explicitly when desired.' : draft.incomeTreatment === 'refund' ? 'Receipt increases cash and restores the selected spending category.' : 'This existing receipt retains its original category effect.'}</p>
      </> : null}
      <label><span>Transaction date · {monthLabel(snapshot.budgetMonth)}</span><input type="date" min={snapshot.budgetMonth + '-01'} max={lastDayOfMonth(snapshot.budgetMonth)} value={draft.date} disabled={saving} onChange={event => update('date', event.target.value)} /></label>
      <label className="transaction-description-field"><span>Description</span><input value={draft.description} maxLength={500} disabled={saving} onChange={event => update('description', event.target.value)} placeholder="What was this transaction for?" /></label>
      {(draft.kind === 'expense' || draft.kind === 'reallocate') ? <label><span>{draft.kind === 'expense' ? 'Budget category' : 'Move from'}</span><select aria-label={draft.kind === 'expense' ? 'Budget category' : 'Move from'} value={draft.fromCategory} disabled={saving} onChange={event => update('fromCategory', event.target.value)}><option value="">Choose category</option>{snapshot.budgets.filter(item => draft.kind === "reallocate" || item.name !== "Unallocated").map(item => <option key={item.name} value={item.name}>{item.name}</option>)}</select></label> : null}
      {(draft.kind === 'reallocate' || draft.kind === 'income' && (draft.incomeTreatment === 'refund' || draft.incomeTreatment === 'legacy')) ? <label><span>{draft.kind === 'income' ? 'Budget category' : 'Move to'}</span><select aria-label={draft.kind === 'income' ? 'Budget category' : 'Move to'} value={draft.toCategory} disabled={saving} onChange={event => update('toCategory', event.target.value)}><option value="">Choose category</option>{snapshot.budgets.filter(item => draft.kind === "reallocate" || item.name !== "Unallocated").map(item => <option key={item.name} value={item.name}>{item.name}</option>)}</select></label> : null}
      {(draft.kind === 'expense' || draft.kind === 'transfer') ? <label><span>{draft.kind === 'expense' ? 'Account' : 'From account'}</span><select aria-label={draft.kind === 'expense' ? 'Account' : 'From account'} value={draft.fromAccount} disabled={saving} onChange={event => { update('fromAccount', event.target.value); if (draft.kind !== 'reallocate') update('currency', snapshot.accounts.find(account => account.name === event.target.value)?.currency ?? 'EUR'); }}><option value="">Choose account</option>{snapshot.accounts.map(item => <option key={item.name} value={item.name}>{item.name} · {item.currency}</option>)}</select></label> : null}
      {(draft.kind === 'income' || draft.kind === 'transfer') ? <label><span>{draft.kind === 'income' ? 'Deposit account' : 'To account'}</span><select aria-label={draft.kind === 'income' ? 'Deposit account' : 'To account'} value={draft.toAccount} disabled={saving} onChange={event => { update('toAccount', event.target.value); if (draft.kind === 'income') update('currency', snapshot.accounts.find(account => account.name === event.target.value)?.currency ?? 'EUR'); }}><option value="">Choose account</option>{snapshot.accounts.map(item => <option key={item.name} value={item.name}>{item.name} · {item.currency}</option>)}</select></label> : null}
      <label><span>{draft.kind === 'reallocate' ? 'Amount · EUR' : 'Amount'}</span><NumericInput aria-label="Amount" value={draft.amount} disabled={saving} onChange={value => update('amount', value)} placeholder="0.00" /></label>
      {draft.kind !== 'reallocate' ? <label><span>Currency</span><select aria-label="Currency" value={draft.currency} disabled={saving} onChange={event => update('currency', event.target.value as Currency)}><option value="EUR">EUR</option><option value="USD">USD</option><option value="PHP">PHP</option></select></label> : null}
      {crossCurrency ? <label><span>Amount received · {destinationAccount?.currency}</span><NumericInput aria-label={'Amount received · ' + destinationAccount?.currency} value={draft.toAmount} disabled={saving} onChange={value => update('toAmount', value)} placeholder="0.00" /></label> : null}
      {(draft.kind === 'expense' || draft.kind === 'income') && snapshot.budgetMonth === currentMonthKey() ? <details className="transaction-details" open={detailsOpen} onToggle={event => setDetailsOpen(event.currentTarget.open)}><summary>More details{draft.correctsTransactionId ? ' · linked correction' : ''}</summary>
      {(draft.kind === 'expense' || draft.kind === 'income') && snapshot.budgetMonth === currentMonthKey() ? <label><span>Correction for earlier transaction (optional)</span><select aria-label="Correction for earlier transaction (optional)" value={draft.correctsTransactionId} disabled={saving} onChange={event => update('correctsTransactionId', event.target.value)}>
        <option value="">New activity</option>{snapshot.recentTransactions.filter(t => t.transactionId && t.date && t.date.slice(0,7) !== snapshot.budgetMonth && !t.canEdit).map(t => <option key={t.transactionId!} value={t.transactionId!}>{t.date} · {t.description}</option>)}
      </select></label> : null}
      </details> : null}
      {amount !== null && amount > 0 ? <div className="transaction-preview" aria-live="polite"><strong>When saved</strong><ul>
        {transactionPreview({ kind: draft.kind, amount, currency: draft.kind === 'reallocate' ? 'EUR' : draft.currency,
          toAmount, fromAccount: sourceAccount, toAccount: destinationAccount,
          fromCategory: draft.fromCategory, toCategory: draft.toCategory, incomeTreatment: draft.incomeTreatment,
          outstandingEur: salaryExpectation?.currency === 'EUR' ? salaryExpectation.outstandingEur : undefined,
          correction: Boolean(draft.correctsTransactionId), editing: Boolean(initial),
        }).map(line => <li key={line}>{line}</li>)}
      </ul></div> : null}
      {!valid ? <ul className="transaction-editor-error" role="status">{!dateInMonth ? <li>Date: choose a date in {monthLabel(snapshot.budgetMonth)}.</li> : null}{!draft.description.trim() || draft.description.length > 500 ? <li>Description: enter 1–500 characters.</li> : null}{amount === null || amount <= 0 ? <li>Amount: enter a value greater than zero with at most two decimal places.</li> : null}{!hasAccounts ? <li>Accounts: select the required account, and different accounts for a transfer.</li> : null}{!hasCategories ? <li>Categories: select the required category, and different categories for a budget move.</li> : null}{crossCurrency && (toAmount === null || toAmount <= 0) ? <li>Amount received: enter a value greater than zero.</li> : null}</ul> : null}
      {initial && draft.baseRevision !== initial.revision ? <div role="alert"><p>Another household member changed this entry. Your draft is still saved. Latest entry: {initial.description} · {currency(initial.amount, initial.currency ?? 'EUR')} · {initial.date}. Compare these with your fields before continuing.</p><button type="button" disabled={saving} onClick={() => { if (window.confirm('Replace your draft fields with the latest saved entry?')) { setDraft(transactionDraft(snapshot, initial)); setDirty(true); setError(''); } }}>Use latest saved entry</button><button type="button" disabled={saving || !valid} onClick={() => { if (window.confirm('Apply your draft to the latest version? Review both entries before saving.')) update('baseRevision', initial.revision ?? ''); }}>Keep my fields for review</button></div> : null}
      {error ? <p className="inline-error transaction-editor-error" role="alert">{error}</p> : null}
      <p role="status">{dirty ? draftSaved ? 'Draft saved on this device. Closing or changing months keeps it.' : 'Draft could not be saved on this device. Keep this page open.' : 'Drafts are restored when you reopen this entry in this month.'}</p>
      <div className="transaction-editor-actions"><button className="text-button" type="button" disabled={saving} onClick={discard}>Discard draft</button><button className="button button-quiet" type="button" disabled={saving} onClick={() => { if (allowDraftNavigation()) onCancel(); }}>Close draft</button><button className="button button-primary" type="submit" disabled={!valid || saving}>{saving ? 'Saving…' : initial ? 'Save changes' : 'Add transaction'}</button>{!initial ? <button className="button button-quiet" type="submit" data-add-another="true" disabled={!valid || saving}>Save and add another</button> : null}</div>
    </form>
  </Panel>;
}

function LedgerPage({ snapshot, onRefresh, mutation, onMutation, user, filter, onSelectMonth }: PageProps & {
  onRefresh: () => Promise<boolean>; mutation: string; onMutation: (value: string) => void;
  user: string; filter: LedgerFilter; onSelectMonth: (month: string) => void;
}) {
  const [query, setQuery] = useState(() => new URLSearchParams(window.location.search).get('search') ?? '');
  const [search, setSearch] = useState(() => new URLSearchParams(window.location.search).get('search') ?? '');
  const [kind, setKind] = useState(() => new URLSearchParams(window.location.search).get('kind') ?? 'All types');
  const [category, setCategory] = useState(filter.category ?? new URLSearchParams(window.location.search).get('category') ?? '');
  const [account, setAccount] = useState(filter.account ?? new URLSearchParams(window.location.search).get('account') ?? '');
  const [allHistory, setAllHistory] = useState(() => new URLSearchParams(window.location.search).get('history') === 'all');
  const [pageNumber, setPageNumber] = useState(1);
  const [pageSize, setPageSize] = useState(50);
  const [sort, setSort] = useState('date');
  const [direction, setDirection] = useState<'asc' | 'desc'>('desc');
  const sortOrder = (key: string): 'ascend' | 'descend' | null => key === sort ? direction === 'asc' ? 'ascend' : 'descend' : null;
  const [rows, setRows] = useState<GatewayTransaction[]>([]);
  const [total, setTotal] = useState(0);
  const [fetching, setFetching] = useState(false);
  const [fetchError, setFetchError] = useState('');
  const [retry, setRetry] = useState(0);
  const [editor, setEditor] = useState<'new' | GatewayTransaction | null>(null);
  const [message, setMessage] = useState('');
  const [pendingDeletes, setPendingDeletes] = useState<Record<string, string>>({});
  const currentMonth = snapshot.budgetHistory?.find(item => item.month === snapshot.budgetMonth)?.status === 'open';
  useEffect(() => { setCategory(filter.category ?? new URLSearchParams(window.location.search).get('category') ?? ''); setAccount(filter.account ?? new URLSearchParams(window.location.search).get('account') ?? ''); setPageNumber(1); }, [filter]);
  useEffect(() => {
    const sync = () => { const params = new URLSearchParams(window.location.search); setQuery(params.get('search') ?? ''); setKind(params.get('kind') ?? 'All types'); setAllHistory(params.get('history') === 'all'); };
    window.addEventListener('presto-ledger-route', sync);
    return () => window.removeEventListener('presto-ledger-route', sync);
  }, []);
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    for (const [key, value] of Object.entries({ search: query, kind: kind === 'All types' ? '' : kind, category, account, history: allHistory ? 'all' : '' })) {
      if (value) params.set(key, value); else params.delete(key);
    }
    window.history.replaceState({}, '', window.location.pathname + '?' + params);
  }, [query, kind, category, account, allHistory]);
  useEffect(() => { const timer = window.setTimeout(() => { setSearch(query); setPageNumber(1); }, 250); return () => window.clearTimeout(timer); }, [query]);
  useEffect(() => { setPageNumber(1); }, [snapshot.budgetMonth]);
  useEffect(() => {
    const controller = new AbortController();
    setFetching(true); setFetchError(''); setRows([]);
    const kinds: Record<string, string> = { Expense: 'expense', Income: 'income', Transfer: 'transfer', 'Budget move': 'reallocate', Reconciliation: 'reconciliation' };
    getTransactions({ month: allHistory ? undefined : snapshot.budgetMonth, search,
      kind: kinds[kind], category, account, page: pageNumber, pageSize, sort, direction }, controller.signal)
      .then(result => {
        if (controller.signal.aborted) return;
        const lastPage = Math.max(1, Math.ceil(result.total / pageSize));
        if (pageNumber > lastPage) setPageNumber(lastPage);
        setRows(result.transactions); setTotal(result.total);
      })
      .catch(error => { if (!controller.signal.aborted) { setFetchError(error instanceof Error ? error.message : 'Could not load transactions.'); setTotal(0); } })
      .finally(() => { if (!controller.signal.aborted) setFetching(false); });
    return () => controller.abort();
  }, [snapshot.budgetMonth, snapshot.generatedAt, search, kind, category, account, allHistory, pageNumber, pageSize, sort, direction, retry]);
  const categoryBalance = snapshot.budgets.find(item => item.name === category);
  const accountBalance = snapshot.accounts.find(item => item.name === account);

  async function saveTransaction(payload: SubmitTransaction, addAnother = false): Promise<TransactionDraft | null> {
    setMessage('');
    onMutation('transaction');
    try {
      if (editor === 'new') {
        const candidates: GatewayTransaction[] = [];
        let candidatePage = 1;
        let totalCandidates = 0;
        do {
          const result = await getTransactions({ month: payload.date.slice(0, 7), account: payload.fromAccount ?? payload.toAccount, page: candidatePage++, pageSize: 100 });
          candidates.push(...result.transactions); totalCandidates = result.total;
          if (!result.transactions.length) break;
        } while (candidates.length < totalCandidates);
        const matches = possibleDuplicates(payload, candidates);
        if (matches.length && !window.confirm('Possible duplicate: ' + matches.map(row => row.date + ' · ' + row.description + ' · ' + currency(row.amount, row.currency ?? 'EUR') + ' · entered by ' + (row.createdBy ?? 'household / import')).join('\n') + '\nSave this as a separate purchase anyway?')) return null;
      }
      if (editor && editor !== 'new') {
        const transactionId = editor.transactionId;
        if (!transactionId) throw new Error('This transaction no longer has an editable database ID.');
        await editTransaction({ ...payload, transactionId }, Number(editor.revision));
      } else {
        await submitTransaction(payload);
      }
      writeTransactionDraft(transactionDraftKey(user, snapshot.budgetMonth, editor && editor !== 'new' ? editor.transactionId ?? 'new' : 'new'), null);
      const nextDraft = addAnother ? { ...transactionDraft(snapshot), kind: payload.kind,
        currency: payload.currency ?? 'EUR', date: payload.date, fromAccount: payload.fromAccount ?? '',
        toAccount: payload.toAccount ?? '', fromCategory: payload.fromCategory ?? '',
        toCategory: payload.toCategory ?? '', incomeTreatment: payload.incomeTreatment ?? 'salary',
        expectedIncomeMonth: payload.expectedIncomeMonth ?? snapshot.budgetMonth } : null;
      if (!addAnother) setEditor(null);
      if (!await onRefresh()) setMessage('Transaction saved. Refresh to load the updated ledger.');
      else setMessage(addAnother ? 'Transaction saved. Ready for the next entry.' : 'Transaction saved.');
      return nextDraft;
    } catch (error) {
      setMessage('Changes were not saved. ' + (error instanceof Error ? error.message : 'Could not save the transaction.') + (editor && editor !== 'new' ? ' Your draft is kept. Refresh and reopen the latest entry to compare it with your draft.' : ' Your draft is kept. Review the fields and try again.'));
      if (editor && editor !== 'new') {
        await onRefresh();
        try {
          let recoveryPage = 1;
          let found = false;
          while (!found) {
            const latest = await getTransactions({ month: snapshot.budgetMonth, page: recoveryPage, pageSize: 100 });
            const updated = latest.transactions.find(item => item.transactionId === editor.transactionId);
            if (updated) { setEditor(updated); found = true; }
            if (found || recoveryPage++ * 100 >= latest.total || !latest.transactions.length) break;
          }
        } catch { /* Keep the saved draft and original error when recovery is offline. */ }
      }
      return null;
    } finally {
      onMutation('');
    }
  }

  async function removeTransaction(item: GatewayTransaction) {
    if (!item.transactionId || !window.confirm('Delete “' + item.description + '”? This will update the account balance and budget totals.')) return;
    setMessage('');
    onMutation('transaction');
    try {
      const operationId = pendingDeletes[item.transactionId] ?? uuidV7();
      setPendingDeletes(current => ({ ...current, [item.transactionId!]: operationId }));
      await deleteTransaction(item.transactionId, operationId);
      writeTransactionDraft(transactionDraftKey(user, item.date?.slice(0, 7) ?? snapshot.budgetMonth, item.transactionId), null);
      if (await onRefresh()) {
        setPendingDeletes(current => { const next = { ...current }; delete next[item.transactionId!]; return next; });
      } else {
        setMessage('Transaction deleted. Refresh to load the updated ledger.');
      }
    } catch (error) {
      setMessage(error instanceof Error ? error.message : 'Could not delete the transaction.');
    } finally {
      onMutation('');
    }
  }

  return (
    <>
      <div className="page-intro"><div><div className="eyebrow">TRANSACTIONS</div><h1>Transactions</h1><p>{snapshot.transactionCount.toLocaleString('en-GB')} transactions · {allHistory ? 'searching all history' : 'viewing ' + monthLabel(snapshot.budgetMonth)}.</p></div><div className="ledger-page-actions"><button className="button button-primary" type="button" disabled={!currentMonth || mutation !== ''} title={currentMonth ? undefined : 'Select an open budget month to add a transaction.'} onClick={() => { setEditor('new'); setMessage(''); }}>＋ Add / resume transaction</button>{!currentMonth ? <small>Select an open month to add a transaction.</small> : null}</div></div>
      {editor ? <TransactionEditor key={snapshot.budgetMonth + (editor === 'new' ? 'new' : editor.transactionId)} user={user} snapshot={snapshot} initial={editor === 'new' ? undefined : editor} saving={mutation === 'transaction'} onCancel={() => setEditor(null)} onSave={saveTransaction} /> : null}
      {categoryBalance ? <Panel><SectionHeading eyebrow="BALANCE BREAKDOWN" title={categoryBalance.name + ' · ' + monthLabel(snapshot.budgetMonth)} />
        {categoryBalance.carryoverEur !== null && categoryBalance.carryoverEur !== undefined && categoryBalance.netSpendingEur !== null && categoryBalance.netSpendingEur !== undefined ? <p>
          Carryover {currency(categoryBalance.carryoverEur)} + allocation {currency(categoryBalance.monthlyEur)} − net spending {currency(categoryBalance.netSpendingEur)} + adjustments ({signedCurrency(categoryBalance.adjustmentEur ?? 0, 'EUR')}) = remaining {currency(categoryBalance.remainingEur)}
        </p> : <p>Frozen remaining balance: {currency(categoryBalance.remainingEur)}. A full component breakdown is unavailable for this historical month.</p>}
        <p>{categoryBalance.name === 'Unallocated' ? 'Adjustments include planned funding, allocations, income settlements, budget moves, and corrections.' : 'Adjustments include budget moves and corrections.'} {allHistory ? 'Transactions below span all history; this balance is for the reporting month.' : 'Transactions below show activity in the reporting month.'}</p>
      </Panel> : null}
      {accountBalance ? <Panel><SectionHeading eyebrow="ACCOUNT DETAIL" title={accountBalance.name} /><p>Current book balance: {currency(accountBalance.balanceNative, accountBalance.currency)} · Recorded reconciliation adjustment: {signedCurrency(accountBalance.reconciliationNative ?? 0, accountBalance.currency)}</p><p>The ledger below shows {allHistory ? 'all recorded activity' : 'activity in ' + monthLabel(snapshot.budgetMonth)} for this account. The current book balance includes activity outside the selected month.</p></Panel> : null}
      {message ? <p className={"ledger-message" + (message.startsWith('Changes were not saved.') ? ' ledger-message-error' : '')} role={message.startsWith('Changes were not saved.') ? 'alert' : 'status'}>{message}</p> : null}
      <Panel>
        <div className="ledger-tools">
          <label className="search-field"><span aria-hidden="true">⌕</span><input value={query} onChange={(event) => setQuery(event.target.value)} aria-label="Search transactions" maxLength={500} placeholder="Search descriptions, accounts, categories" /></label>
          <Select
            className="ledger-type-select"
            value={kind}
            onChange={value => { setKind(value); setPageNumber(1); }}
            aria-label="Filter by transaction type"
            options={['All types', 'Expense', 'Income', 'Transfer', 'Budget move', 'Reconciliation'].map((value) => ({ value, label: value }))}
          />
          <label className="check-switch"><input type="checkbox" checked={allHistory} onChange={event => { setAllHistory(event.target.checked); setPageNumber(1); }} />All history</label>
          <Select aria-label="Filter by category" placeholder="All categories" allowClear value={category || undefined} onChange={value => { setCategory(value ?? ''); setPageNumber(1); }} options={snapshot.budgets.map(item => ({ value: item.name, label: item.name }))} />
          <Select aria-label="Filter by account" placeholder="All accounts" allowClear value={account || undefined} onChange={value => { setAccount(value ?? ''); setPageNumber(1); }} options={snapshot.accounts.map(item => ({ value: item.name, label: item.name }))} />
          <span className="panel-caption">{fetching ? 'Loading…' : total.toLocaleString('en-GB') + ' matching transactions'}</span>
          <button type="button" className="text-button" onClick={() => { setQuery(''); setSearch(''); setKind('All types'); setCategory(''); setAccount(''); setPageNumber(1); }}>Clear filters</button>
        </div>
        {fetchError ? <p className="inline-error" role="alert">{fetchError} <button type="button" className="text-button" onClick={() => setRetry(value => value + 1)}>Retry</button></p> : null}
        <Table<GatewayTransaction>
          loading={fetching}
          onChange={(_, __, sorting, extra) => {
            if (extra.action !== 'sort') return;
            const selected = Array.isArray(sorting) ? sorting[0] : sorting;
            setSort(selected?.order ? String(selected.columnKey) : 'date');
            setDirection(selected?.order === 'ascend' ? 'asc' : 'desc'); setPageNumber(1);
          }}
          className="sortable-table ledger-table"
          rowKey={item => String(item.transactionId || item.row)}
          dataSource={rows}
          pagination={{ current: pageNumber, pageSize, total, showSizeChanger: true, pageSizeOptions: [25, 50, 100], onChange: (page, size) => { setPageNumber(size !== pageSize ? 1 : page); setPageSize(size); } }}
          tableLayout="fixed"
          scroll={{ x: '100%' }}
          locale={{ emptyText: <EmptyState title={fetching ? 'Loading transactions' : fetchError ? 'Transactions unavailable' : search || category || account || kind !== 'All types' ? 'No matching transactions' : 'No transactions recorded'} detail={fetchError ? 'Retry to load the ledger.' : fetching ? 'Fetching the selected page.' : search || category || account || kind !== 'All types' ? 'Clear filters or search all history.' : 'This reporting period has no recorded entries.'} /> }}
          columns={[
            { title: 'Date', key: 'date', width: '10%', sorter: true, sortOrder: sortOrder('date'), render: (_, item) => transactionDateLabel(item) },
            { title: 'Description', key: 'description', width: '22%', sorter: true, sortOrder: sortOrder('description'), render: (_, item) => <div className="ledger-description">
              <strong>{item.description || '—'}</strong>
              <small className="ledger-entry-meta">Entered by {item.createdBy ?? 'household / import'}{item.createdAt ? ' · ' + dateLabel(item.createdAt.slice(0, 10)) : ''}</small>
              {item.transactionId ? <><small className="ledger-entry-id" title={item.transactionId}>Entry {item.transactionId.slice(0, 8)}</small><TransactionHistory id={item.transactionId} revision={item.revision ?? '1'} /></> : null}
            </div> },
            { title: 'Account', key: 'account', width: '15%', render: (_, item) => transactionDimension(item, 'account') },
            { title: 'Category', key: 'category', width: '14%', render: (_, item) => transactionDimension(item, 'category') },
            { title: 'Type', key: 'type', width: '14%', sorter: true, sortOrder: sortOrder('type'), render: (_, item) => {
              const kindName = transactionKind(item);
              const tone = kindName === 'Expense' ? 'bad' : kindName === 'Income' ? 'good' : kindName === 'Transfer' ? 'info' : kindName === 'Budget move' ? 'budget' : 'warn';
              return <Tag tone={tone}>{kindName}</Tag>;
            } },
            { title: 'Amount', key: 'amount', width: '13%', align: 'right', className: 'align-right', sorter: true, sortOrder: sortOrder('amount'), render: (_, item) => {
              const kindName = transactionKind(item);
              const positive = kindName === 'Income' || (kindName === 'Reconciliation' && numeric(item.amount) < 0);
              return <span className={positive ? 'value-positive' : ''}>{positive ? '+' : kindName === 'Expense' ? '−' : ''}{currency(Math.abs(numeric(item.amount)), item.currency || 'EUR')}</span>;
            } },
            { title: 'Actions', key: 'actions', width: '12%', className: 'ledger-row-actions', render: (_, item) => item.canEdit && item.date?.slice(0, 7) === snapshot.budgetMonth ? <><button type="button" disabled={mutation !== '' || fetching} onClick={() => { setEditor(item); setMessage(''); }}>Edit</button><button type="button" disabled={mutation !== '' || fetching} onClick={() => void removeTransaction(item)}>Delete</button></> : item.canEdit && item.date ? <button type="button" disabled={mutation !== ''} onClick={() => onSelectMonth(item.date!.slice(0, 7))}>Open month ↗</button> : <span>Closed or imported</span> },
          ]}
        />
      </Panel>
    </>
  );
}

function compactCurrency(value: number, code: string) {
  try {
    return new Intl.NumberFormat('en-IE', {
      style: 'currency', currency: code, notation: 'compact', maximumFractionDigits: 1,
    }).format(value);
  } catch {
    return code + ' ' + value.toLocaleString('en-GB', { maximumFractionDigits: 0 });
  }
}

function AccountHistoryPage({ snapshot }: PageProps) {
  const [selected, setSelected] = useState('compare');
  const recordedAccounts = new Map<string, { name: string; currency: Currency }>();
  snapshot.accountHistory.forEach((day) => day.accounts.forEach((item) => {
    recordedAccounts.set(item.accountId, { name: item.name, currency: item.currency });
  }));
  const choices = [
    ...(recordedAccounts.size > 1
      ? [{ key: 'compare', label: 'Compare accounts', currency: 'EUR' as Currency }]
      : []),
    ...(snapshot.accountHistory.some((day) => day.totalEur !== null)
      ? [{ key: 'total', label: 'Household total', currency: 'EUR' as Currency }]
      : []),
    ...Array.from(recordedAccounts, ([key, account]) => ({ key, label: account.name, currency: account.currency })),
  ];
  const active = choices.find((choice) => choice.key === selected) ?? choices[0];
  const comparisonSeries: AccountHistoryComparisonSeries[] = Array.from(recordedAccounts, ([key, account]) => ({
    key,
    label: account.name,
    points: snapshot.accountHistory.flatMap((day) => {
      const balance = day.accounts.find((item) => item.accountId === key);
      if (!balance || balance.balanceEur === null) return [];
      return [{ date: day.date, value: balance.balanceEur, balanceEur: balance.balanceEur }];
    }),
  })).filter((account) => account.points.length > 0);
  if (active?.key === 'compare') {
    const dates = Array.from(new Set(comparisonSeries.flatMap((account) => account.points.map((point) => point.date)))).sort();
    return <>
      <div className="page-intro"><div><div className="eyebrow">ACCOUNT BALANCES</div><h1>Account history</h1><p>Compare recorded account balances over time. Each account is valued in EUR using a prior ECB rate, falling back to imported historical rates when needed.</p></div></div>
      <Panel>
        <SectionHeading eyebrow="HISTORICAL VIEW" title="Compare accounts" detail="Compare accounts side by side in EUR across their recorded balance checkpoints." />
        <div className="history-series" role="group" aria-label="Choose account history series">
          {choices.map((choice) => <button key={choice.key} type="button" className={'history-series-button ' + (active.key === choice.key ? 'history-series-active' : '')} aria-pressed={active.key === choice.key} onClick={() => setSelected(choice.key)}>{choice.label}<small>{choice.currency}</small></button>)}
        </div>
        {dates.length ? <div className="history-summary">
          <div className="history-summary-card history-summary-current"><span>Accounts compared</span><strong>{comparisonSeries.length}</strong><small>Showing accounts with at least one EUR valued checkpoint</small></div>
          <div className="history-summary-card"><span>Checkpoint dates</span><strong>{dates.length}</strong><small>Dates with one or more recorded account balances</small></div>
          <div className="history-summary-card"><span>Recorded period</span><strong>{dateLabel(dates[0])}</strong><small>Through {dateLabel(dates[dates.length - 1])}</small></div>
        </div> : null}
      </Panel>
      <Panel>
        <SectionHeading eyebrow="BALANCE TREND" title="Accounts over time" detail="Each bar stacks the available account balances in EUR at a checkpoint date. Missing values are omitted. Hover or use the arrow keys to inspect accounts." />
        {comparisonSeries.length ? <AccountHistoryComparisonChart series={comparisonSeries} formatDate={dateLabel} formatCurrency={currency} formatCompactCurrency={compactCurrency} />
          : <EmptyState title="No comparable account history" detail="There are no recorded checkpoints with an available EUR value for these accounts." />}
      </Panel>
      <Panel>
        <SectionHeading eyebrow="NUMERICAL HISTORY" title="Checkpoint comparison" detail="EUR equivalents for each account on its recorded checkpoint dates, listed newest first." />
        <Table
          className="sortable-table history-table history-comparison-table"
          rowKey="date"
          dataSource={snapshot.accountHistory.filter(day => dates.includes(day.date))}
          pagination={false}
          scroll={{ x: 'max-content' }}
          locale={{ emptyText: <EmptyState title="No numerical history" detail="Recorded checkpoints for this comparison will appear here." /> }}
          columns={[
            { title: 'Date', key: 'date', defaultSortOrder: 'descend', sorter: (a, b) => a.date.localeCompare(b.date), render: (_, day) => dateLabel(day.date) },
            ...comparisonSeries.map(account => ({
              title: account.label + ' · EUR', key: account.key, align: 'right' as const,
              sorter: (a: FinanceSnapshot['accountHistory'][number], b: FinanceSnapshot['accountHistory'][number]) => compareNullableNumbers(a.accounts.find(item => item.accountId === account.key)?.balanceEur, b.accounts.find(item => item.accountId === account.key)?.balanceEur),
              render: (_: unknown, day: FinanceSnapshot['accountHistory'][number]) => {
                const balance = day.accounts.find(item => item.accountId === account.key)?.balanceEur;
                return balance == null ? '—' : currency(balance, 'EUR');
              },
            })),
          ]}
        />
      </Panel>
    </>;
  }
  const points: AccountHistoryDatum[] = active
    ? snapshot.accountHistory.flatMap((day) => {
      if (active.key === 'total') return day.totalEur === null ? [] : [{ date: day.date, value: day.totalEur, balanceEur: day.totalEur }];
      const balance = day.accounts.find((item) => item.accountId === active.key);
      return balance ? [{ date: day.date, value: balance.balanceNative, balanceEur: balance.balanceEur }] : [];
    })
    : [];
  const first = points[0];
  const latest = points[points.length - 1];
  const change = first && latest ? latest.value - first.value : 0;
  const changePercent = first && Math.abs(first.value) > 0.005 ? change / Math.abs(first.value) * 100 : null;

  return <>
    <div className="page-intro"><div><div className="eyebrow">ACCOUNT BALANCES</div><h1>Account history</h1><p>Recorded balance checkpoints through {monthLabel(snapshot.budgetMonth)}, with foreign currencies valued in EUR at the rate available on each date.</p></div></div>
    <Panel>
      <SectionHeading eyebrow="HISTORICAL VIEW" title={active?.label ?? 'Recorded account values'} detail="Choose a series to inspect its movement and checkpoint values." />
      {choices.length ? <div className="history-series" role="group" aria-label="Choose account history series">
        {choices.map((choice) => <button key={choice.key} type="button" className={'history-series-button ' + (active?.key === choice.key ? 'history-series-active' : '')} aria-pressed={active?.key === choice.key} onClick={() => setSelected(choice.key)}>{choice.label}<small>{choice.currency}</small></button>)}
      </div> : null}
      {latest ? <div className="history-summary">
        <div className="history-summary-card history-summary-current"><span>Latest recorded value</span><strong>{currency(latest.value, active?.currency)}</strong><small>{dateLabel(latest.date)}{active?.key !== 'total' ? ' · ' + (latest.balanceEur === null ? 'EUR value unavailable' : currency(latest.balanceEur, 'EUR')) : ''}</small></div>
        <div className="history-summary-card"><span>Change across recorded period</span><strong className={change < 0 ? 'value-negative' : 'value-positive'}>{signedCurrency(change, active?.currency ?? 'EUR')}</strong><small>{changePercent === null ? 'Compared with the first checkpoint' : (changePercent > 0 ? '+' : '') + changePercent.toFixed(1) + '% from ' + dateLabel(first?.date ?? null)}</small></div>
        <div className="history-summary-card"><span>Recorded checkpoints</span><strong>{points.length}</strong><small>{points.length ? dateLabel(first.date) + ' to ' + dateLabel(latest.date) : 'No checkpoint dates available'}</small></div>
      </div> : null}
    </Panel>
    <Panel>
      <SectionHeading eyebrow="BALANCE TREND" title={active ? active.label + ' over time' : 'No account history available'} detail={active?.key === 'total' ? 'The household total includes dates with a recorded value for every active account. Hover over the chart or use the arrow keys to inspect values.' : 'Each point is an archived account balance. EUR equivalents use the latest available exchange rate on that date. Hover over the chart or use the arrow keys to inspect values.'} />
      {points.length ? <AccountHistoryChart points={points} label={active?.label ?? 'Account'} code={active?.currency ?? 'EUR'} formatDate={dateLabel} formatCurrency={currency} formatCompactCurrency={compactCurrency} />
        : <EmptyState title="No historical balance checkpoints" detail="The database does not contain recorded account values for this reporting period." />}
    </Panel>
    <Panel>
      <SectionHeading eyebrow="NUMERICAL HISTORY" title={active ? active.label + ' checkpoints' : 'Checkpoint values'} detail="Recorded values used to draw the chart, listed newest first." />
      <Table<AccountHistoryDatum>
        key={active?.key}
        className="sortable-table history-table"
        rowKey="date"
        dataSource={points}
        pagination={false}
        scroll={{ x: 'max-content' }}
        locale={{ emptyText: <EmptyState title="No numerical history" detail="Recorded checkpoints for this view will appear here." /> }}
        columns={[
          { title: 'Date', key: 'date', defaultSortOrder: 'descend', sorter: (a, b) => a.date.localeCompare(b.date), render: (_, point) => dateLabel(point.date) },
          { title: 'Balance · ' + (active?.currency ?? 'EUR'), key: 'balance', align: 'right', sorter: (a, b) => a.value - b.value, render: (_, point) => currency(point.value, active?.currency) },
          { title: 'EUR equivalent', key: 'eur', align: 'right', hidden: active?.key === 'total', sorter: (a, b) => compareNullableNumbers(a.balanceEur, b.balanceEur), render: (_, point) => point.balanceEur === null ? '—' : currency(point.balanceEur, 'EUR') },
        ]}
      />
    </Panel>
  </>;
}

export default function WebApp() {
  const [email, setEmail] = useState('');
  const [page, setPage] = useState<PageKey>(() => pageFromPath(window.location.pathname));
  const pageRef = useRef(page);
  pageRef.current = page;
  const [month, setMonth] = useState(() => { const value = new URLSearchParams(window.location.search).get('month'); return value && /^\d{4}-(0[1-9]|1[0-2])$/.test(value) ? value : currentMonthKey(); });
  const [ledgerFilter, setLedgerFilter] = useState<LedgerFilter>({});
  const [snapshot, setSnapshot] = useState<FinanceSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [refreshFailed, setRefreshFailed] = useState(false);
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 30000); return () => window.clearInterval(timer); }, []);
  const [connecting, setConnecting] = useState(false);
  const [mutation, setMutation] = useState('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [ignoreReallocation, setIgnoreReallocation] = useState(false);
  const [reconciliationDraftFailed, setReconciliationDraftFailed] = useState(false);
  const [actualBalances, updateActualBalances] = useState<Record<string, string>>({});
  const actualBalancesRef = useRef<Record<string, string>>({});
  const draftKeyRef = useRef<string | null>(null);
  const setActualBalances = useCallback((updater: (current: Record<string, string>) => Record<string, string>) => {
    const balances = updater(actualBalancesRef.current);
    actualBalancesRef.current = balances;
    updateActualBalances(balances);
    if (draftKeyRef.current) {
      try {
        setReconciliationDraftFailed(!writeReconciliationDraft(window.localStorage, draftKeyRef.current, balances));
      } catch {
        setReconciliationDraftFailed(true);
      }
    }
  }, []);
  const [reconciliationPreview, setReconciliationPreview] = useState<ReconciliationPreview | null>(null);
  const [reconciliationLoading, setReconciliationLoading] = useState(false);
  const reconciliationSequence = useRef(0);
  const requestSequence = useRef(0);

  useEffect(() => {
    const syncDraft = (event: StorageEvent) => {
      if (!draftKeyRef.current || (event.key !== null && event.key !== draftKeyRef.current)) return;
      try {
        if (event.storageArea !== window.localStorage) return;
        const balances = readReconciliationDraft(window.localStorage, draftKeyRef.current);
        actualBalancesRef.current = balances;
        updateActualBalances(balances);
      } catch {
        // Keep current entries if storage access is denied.
      }
    };
    window.addEventListener('storage', syncDraft);
    return () => window.removeEventListener('storage', syncDraft);
  }, []);

  const navigateTo = useCallback((nextPage: PageKey, selectedMonth?: string) => {
    if (!allowDraftNavigation()) return;
    if (selectedMonth) setMonth(selectedMonth);
    const nextPath = pagePaths[nextPage];
    if (window.location.pathname !== nextPath) window.history.pushState({}, '', nextPath + window.location.search);
    setPage(nextPage);
  }, []);

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    params.set('month', month);
    window.history.replaceState({}, '', window.location.pathname + '?' + params);
  }, [month]);
  const selectMonth = (selectedMonth: string) => { if (allowDraftNavigation()) setMonth(selectedMonth); };
  const drillDown = (filter: LedgerFilter) => {
    if (!allowDraftNavigation()) return;
    const params = new URLSearchParams(window.location.search);
    for (const key of ['account', 'category']) params.delete(key);
    if (filter.account) params.set('account', filter.account);
    if (filter.category) params.set('category', filter.category);
    window.history.pushState({}, '', pagePaths.ledger + '?' + params);
    setLedgerFilter(filter); setPage('ledger');
  };
  const navigateMonth = navigateTo;
  useEffect(() => {
    const syncRoute = () => {
      const nextPage = pageFromPath(window.location.pathname);
      if (nextPage !== pageRef.current && !allowDraftNavigation()) {
        window.history.replaceState({}, '', pagePaths[pageRef.current]);
        return;
      }
      const canonicalPath = pagePaths[nextPage];
      if (window.location.pathname !== canonicalPath) {
        window.history.replaceState({}, '', canonicalPath + window.location.search + window.location.hash);
      }
      const params = new URLSearchParams(window.location.search);
      const selected = params.get("month");
      if (selected && /^\d{4}-(0[1-9]|1[0-2])$/.test(selected)) setMonth(selected);
      setLedgerFilter({ category: params.get("category") ?? undefined, account: params.get("account") ?? undefined });
      window.dispatchEvent(new Event("presto-ledger-route"));
      setPage(nextPage);
    };

    window.addEventListener('popstate', syncRoute);
    syncRoute();
    return () => window.removeEventListener('popstate', syncRoute);
  }, []);

  useEffect(() => {
    const query = new URLSearchParams(window.location.search);
    if (query.get('auth_error') === 'pocket_id_login_failed') {
      setError('Pocket ID sign-in did not complete. Check that your account is allowed, then try again.');
      window.history.replaceState({}, '', window.location.pathname);
    } else if (query.has('auth_error')) {
      setError('Pocket ID sign-in did not complete. Check the callback and household account, then try again.');
      window.history.replaceState({}, '', window.location.pathname);
    }
    getServerSession()
      .then((session) => {
        setEmail(session.pocketAuthenticated ? session.pocketUsername ?? 'Household' : '');
      })
      .catch((sessionError) => setError(sessionError instanceof Error ? sessionError.message : String(sessionError)));
  }, []);

  const loadSnapshot = useCallback(async (selectedMonth: string) => {
    const sequence = ++requestSequence.current;
    setLoading(true);
    setError('');
    try {
      const result = await getFinanceSnapshot(selectedMonth + '-01', 200);
      if (sequence !== requestSequence.current) return false;
      setSnapshot(result); setRefreshFailed(false);
      return true;
    } catch (loadError) {
      if (sequence === requestSequence.current) {
        setRefreshFailed(true);
        setError(loadError instanceof Error ? loadError.message : String(loadError));
      }
      return false;
    } finally {
      if (sequence === requestSequence.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (email && page !== 'reconcile') void loadSnapshot(month);
  }, [email, month, page, loadSnapshot]);

  const refreshReconciliation = useCallback(async () => {
    const sequence = ++reconciliationSequence.current;
    setReconciliationLoading(true);
    setError('');
    try {
      const now = new Date();
      const preview = await getReconciliationPreview(madridDateKey(), month < currentMonthKey() ? month : undefined);
      if (sequence !== reconciliationSequence.current) return false;
      const draftKey = reconciliationDraftKey(email, preview.closingDate);
      if (draftKeyRef.current !== draftKey) {
        let balances: Record<string, string> = {};
        try {
          // Preserve drafts created before local storage autosaving was introduced.
          let persisted = true;
          if (window.localStorage.getItem(draftKey) === null) {
            balances = readReconciliationDraft(window.sessionStorage, draftKey);
            persisted = writeReconciliationDraft(window.localStorage, draftKey, balances);
          } else {
            balances = readReconciliationDraft(window.localStorage, draftKey);
          }
          setReconciliationDraftFailed(!persisted);
          if (persisted) window.sessionStorage.removeItem(draftKey);
        } catch {
          setReconciliationDraftFailed(true);
          try { balances = readReconciliationDraft(window.sessionStorage, draftKey); } catch { /* Retain in-memory fields. */ }
        }
        draftKeyRef.current = draftKey;
        actualBalancesRef.current = balances;
        updateActualBalances(balances);
      }
      setReconciliationPreview(preview);
      return true;
    } catch (reason) {
      if (sequence === reconciliationSequence.current) {
        setReconciliationPreview(null);
        setError(reason instanceof Error ? reason.message : String(reason));
      }
      return false;
    } finally {
      if (sequence === reconciliationSequence.current) setReconciliationLoading(false);
    }
  }, [email, month]);

  useEffect(() => {
    if (email && page === 'reconcile') void refreshReconciliation();
  }, [email, page, refreshReconciliation]);

  useEffect(() => {
    const expired = () => {
      requestSequence.current += 1;
      reconciliationSequence.current += 1;
      setEmail('');
      setSnapshot(null);
      setReconciliationPreview(null);
      setLoading(false);
      setReconciliationLoading(false);
      draftKeyRef.current = null;
      actualBalancesRef.current = {};
      updateActualBalances({});
      setError('Your session expired or was signed out. Sign in again. Saved drafts are kept on this device.');
    };
    window.addEventListener('presto-session-expired', expired);
    return () => window.removeEventListener('presto-session-expired', expired);
  }, []);

  useEffect(() => {
    if (!email) return;
    let active = true;
    let checking = false;
    const check = async () => {
      if (checking || document.visibilityState === 'hidden') return;
      checking = true;
      try {
        const session = await getServerSession();
        if (active && !session.pocketAuthenticated) invalidateServerSession();
      } catch { /* A network failure does not imply that authentication expired. */ }
      finally { checking = false; }
    };
    const timer = window.setInterval(() => void check(), 60_000);
    const visible = () => { void check(); };
    window.addEventListener('focus', visible);
    document.addEventListener('visibilitychange', visible);
    return () => {
      active = false;
      window.clearInterval(timer);
      window.removeEventListener('focus', visible);
      document.removeEventListener('visibilitychange', visible);
    };
  }, [email]);

  function connectGoogle() {
    if (!allowDraftNavigation()) return;
    setConnecting(true);
    setError('');
    startPocketIdLogin();
  }

  function connectPocketId() {
    setConnecting(true);
    setError('');
    startPocketIdLogin();
  }

  async function disconnectGoogle() {
    if (!allowDraftNavigation()) return;
    try {
      await signOutFromServer();
    } catch (signOutError) {
      setError(signOutError instanceof Error ? signOutError.message : String(signOutError));
      return;
    }
    requestSequence.current += 1;
    setEmail('');
    setSnapshot(null);
    reconciliationSequence.current += 1;
    setReconciliationPreview(null);
    draftKeyRef.current = null;
    actualBalancesRef.current = {};
    updateActualBalances({});
    setError('');
    setNotice('Signed out of Pocket ID.');
  }

  async function submitReconciliation() {
    if (!reconciliationPreview) return;
    const now = new Date();
    const today = madridDateKey();
    if (reconciliationPreview.observedDate !== today) {
      await refreshReconciliation();
      setError('The date changed. Review the refreshed balance check before posting.');
      return;
    }
    const accounts = reconciliationPreview.accounts.map((account) => ({
      name: account.name,
      actualBalance: parseAmount(actualBalances[account.name] ?? ''),
      transactionId: uuidV7(),
    }));
    const missing = accounts.find((account) => account.actualBalance === null);
    if (missing) {
      setError('Enter the statement balance for ' + missing.name + '.');
      return;
    }

    setMutation('reconcile');
    setError('');
    try {
      await reconcileAccounts({
        date: reconciliationPreview.closingDate,
        accounts: accounts.map((account) => ({
          name: account.name,
          actualBalance: account.actualBalance as number,
          transactionId: account.transactionId,
        })),
      }, reconciliationPreview);
      setActualBalances(() => ({}));
      setNotice('Accounts reconciled for ' + dateLabel(reconciliationPreview.closingDate) + ' using current balances.');
      await refreshReconciliation();
    } catch (submitError) {
      setError(submitError instanceof Error ? submitError.message : String(submitError));
    } finally {
      setMutation('');
    }
  }

  function signOutButton() {
    return <button className="button button-quiet" type="button" onClick={disconnectGoogle}>Sign out</button>;
  }

  const monthControl = page === 'overview' || page === 'budgets' || page === 'history' || page === 'ledger' || page === 'reconcile'
    ? <label className="month-control"><span>Reporting month</span><DatePicker
      selected={monthDate(month)}
      disabled={mutation !== ''}
      onChange={(date: Date | null) => {
        if (date) selectMonth(date.getFullYear() + '-' + String(date.getMonth() + 1).padStart(2, '0'));
      }}
      dateFormat="MMMM yyyy"
      maxDate={monthDate(currentMonthKey())}
      showMonthYearPicker
      showPopperArrow={false}
      popperPlacement="bottom-end"
      className="month-picker-input"
      aria-label="Reporting month"
    /></label>
    : null;

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark">p</span><span>presto<span className="brand-light"> / finance</span></span></div>
        <div className="workspace-label">WORKSPACE</div>
        <nav className="side-nav" aria-label="Main navigation">
          {navigation.map((item) => <button key={item.key} type="button" disabled={mutation !== ''} className={'nav-link ' + (page === item.key ? 'nav-link-active' : '')} aria-current={page === item.key ? 'page' : undefined} onClick={() => navigateTo(item.key)}><span className="nav-icon" aria-hidden="true">{item.icon}</span><span>{item.label}</span>{item.key === 'reconcile' && snapshot && !snapshot.reconciliationStatus.batch?.isComplete ? <i className="nav-alert" /> : null}</button>)}
        </nav>
        <div className="sidebar-spacer" />
        {email ? (
          <div className="connection-card">
            <div className="connection-top"><span className="live-dot" /><span>HOUSEHOLD FINANCE</span><button type="button" onClick={() => void loadSnapshot(month)} aria-label="Refresh finance data" disabled={loading}>↻</button></div>
            <strong>{email}</strong>
            <small>{loading ? 'Updating data…' : snapshot ? 'Synced ' + dateLabel(snapshot.generatedAt.slice(0, 10)) : 'Connected · waiting for data'}</small>
          </div>
        ) : (
          <div className="connection-card connection-card-muted"><div className="connection-top"><span className="offline-dot" /><span>NOT CONNECTED</span></div><strong>Pocket ID</strong><small>Sign in to view household finance data.</small></div>
        )}
        <div className="sidebar-foot"><span>PRESTO PRESUPUESTO</span><span>Analysis workspace</span></div>
      </aside>
      <main className="main-area">
        <header className="topbar">
          <div className="breadcrumb"><span>Finance</span><i>/</i><strong>{navigation.find((item) => item.key === page)?.label}</strong></div>
          <div className="top-actions">
            {monthControl}
            {email ? <>{snapshot && page !== 'reconcile' ? <small role="status">Updated {new Date(snapshot.generatedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}{refreshFailed || now - Date.parse(snapshot.generatedAt) > 300000 ? ' · May be out of date — refresh' : ''}</small> : null}<button className="button button-quiet refresh-button" type="button" onClick={() => void (page === 'reconcile' ? refreshReconciliation() : loadSnapshot(month))} disabled={loading || reconciliationLoading}>{loading || reconciliationLoading ? 'Updating…' : '↻ Refresh'}</button><button className="button button-quiet" type="button" onClick={() => void connectGoogle()} disabled={connecting}>{connecting ? 'Connecting…' : 'Reconnect'}</button>{signOutButton()}</> : null}
          </div>
        </header>
        <div className="content">
          {error ? <div className="notice notice-error" role="alert"><span>!</span><p>{error}</p><button type="button" onClick={() => setError('')} aria-label="Dismiss error">×</button></div> : null}
          {email && page === 'reconcile' && reconciliationDraftFailed ? <div className="notice notice-error" role="alert"><span>!</span><p>Balance drafts could not be stored on this device. Keep a copy before leaving or reloading.</p></div> : null}
          {notice ? <div className="notice notice-success" role="status"><span>✓</span><p>{notice}</p><button type="button" onClick={() => setNotice('')} aria-label="Dismiss message">×</button></div> : null}
          {!email ? (
            <div className="welcome-grid">
              <div className="welcome-copy">
                <div className="eyebrow">YOUR FINANCIAL CONTROL ROOM</div>
                <h1>See the whole month.<br /><em>Close it with confidence.</em></h1>
                <p>A focused workspace for reviewing budgets, matching real account balances, and understanding where the money went.</p>
                <div className="welcome-points"><span><i>01</i> Monthly budget analysis</span><span><i>02</i> Account reconciliation</span><span><i>03</i> Clear transaction review</span></div>
              </div>
              <div className="login-card">
                <div className="login-orbit"><span>€</span><i>↗</i><b>✓</b></div>
                <div className="eyebrow">PRIVATE HOUSEHOLD DATA</div>
                <h2>Connect household finance</h2>
                <p>Sign in with Pocket ID to load household budgets, transactions and balances.</p>
                <button className="button button-primary login-button" type="button" disabled={connecting} onClick={connectPocketId}>{connecting ? 'Connecting…' : 'Continue with Pocket ID'} <span>→</span></button>
                <small>Only the Pocket ID accounts tim and jopi can open this workspace.</small>
              </div>
            </div>
          ) : page === 'reconcile' ? (
            reconciliationPreview ? <ReconciliationPage user={email} onDrillDown={filter => { setMonth(reconciliationPreview.closingDate.slice(0, 7)); drillDown(filter); }} preview={reconciliationPreview} actualBalances={actualBalances} setActualBalances={setActualBalances} mutation={mutation} loading={reconciliationLoading} onSubmit={() => void submitReconciliation()} onRefresh={() => void refreshReconciliation()} />
              : <div className="loading-state">{reconciliationLoading ? <><span className="loading-spinner" /><strong>Loading balance check</strong></> : <><strong>Balance check unavailable</strong><button className="button button-primary" type="button" onClick={() => void refreshReconciliation()}>Try again</button></>}</div>
          ) : loading && (!snapshot || snapshot.budgetMonth !== month) ? (
            <div className="loading-state"><span className="loading-spinner" /><strong>Loading finance data</strong><p>Fetching calculated budgets, accounts and recent activity.</p></div>
          ) : snapshot && snapshot.budgetMonth === month ? (
            <>
              {page === 'overview' ? <OverviewPage onDrillDown={drillDown} onRefresh={() => loadSnapshot(month)} snapshot={snapshot} onSelectMonth={selectMonth} onNavigate={navigateTo} ignoreReallocation={ignoreReallocation} onToggleReallocation={() => setIgnoreReallocation((value) => !value)} /> : null}
              {page === 'budgets' ? <BudgetsPage onNavigate={navigateMonth} onDrillDown={drillDown} snapshot={snapshot} ignoreReallocation={ignoreReallocation} onSelectMonth={selectMonth} onToggleReallocation={() => setIgnoreReallocation((value) => !value)} onRefresh={() => loadSnapshot(month)} onMutation={setMutation} mutation={mutation} /> : null}
              {page === 'history' ? <AccountHistoryPage snapshot={snapshot} /> : null}
              {page === 'ledger' ? <LedgerPage onSelectMonth={selectMonth} user={email} filter={ledgerFilter} snapshot={snapshot} onRefresh={() => loadSnapshot(month)} mutation={mutation} onMutation={setMutation} /> : null}
              <footer className="content-footer"><span>Household finance · {snapshot.transactionCount.toLocaleString('en-GB')} transactions</span><span>Updated {dateLabel(snapshot.generatedAt.slice(0, 10))}</span></footer>
            </>
          ) : email ? <div className="loading-state"><strong>No finance data loaded</strong><p>Refresh the connection or check that this account has access to the finance service.</p><button className="button button-primary" type="button" onClick={() => void loadSnapshot(month)}>Try again</button></div> : null}
        </div>
      </main>
    </div>
  );
}
