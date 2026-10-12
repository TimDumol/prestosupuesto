-- Optimistic concurrency tokens for account-ledger mutations. Imported
-- balances start at revision zero; every new live transaction or
-- reconciliation increments the affected account's revision atomically.
ALTER TABLE finance_accounts
    ADD COLUMN ledger_revision INTEGER NOT NULL DEFAULT 0 CHECK (ledger_revision >= 0);
