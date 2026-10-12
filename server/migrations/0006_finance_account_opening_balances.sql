-- Preserve the explicitly entered opening balances as account state rather
-- than treating them as ordinary ledger transactions.
CREATE TABLE IF NOT EXISTS finance_account_opening_balances (
    source_workbook_id TEXT NOT NULL,
    source_sheet TEXT NOT NULL,
    source_row INTEGER NOT NULL,
    account_id TEXT NOT NULL REFERENCES finance_accounts(id),
    effective_date TEXT NOT NULL CHECK (effective_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    balance_decimal TEXT NOT NULL,
    original_row_fingerprint TEXT NOT NULL,
    source_row_json TEXT NOT NULL,
    PRIMARY KEY (source_workbook_id, source_sheet, source_row),
    UNIQUE (source_workbook_id, source_sheet, account_id, effective_date)
);

CREATE INDEX IF NOT EXISTS finance_account_opening_balances_account_date
    ON finance_account_opening_balances(account_id, effective_date);
