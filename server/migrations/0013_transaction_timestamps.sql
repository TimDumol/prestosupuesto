-- Local posting dates remain authoritative for budgets and FX. Occurrence times
-- are UTC timestamps; historical/date-only transactions remain NULL.
ALTER TABLE finance_transactions ADD COLUMN occurred_at TEXT;
CREATE INDEX finance_transactions_occurred_at ON finance_transactions(occurred_at);
