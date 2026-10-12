-- Preserve dated actual-balance checkpoints from Accounts!W:BE, including
-- formula/literal/blank/error source state and the original date header.
CREATE TABLE IF NOT EXISTS finance_account_balance_checkpoints (
    source_workbook_id TEXT NOT NULL,
    source_sheet TEXT NOT NULL,
    source_row INTEGER NOT NULL,
    source_column TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES finance_accounts(id),
    checkpoint_date TEXT,
    balance_decimal TEXT,
    source_state TEXT NOT NULL CHECK (source_state IN ('formula', 'literal', 'blank', 'error')),
    source_header_json TEXT NOT NULL,
    source_cell_json TEXT NOT NULL,
    PRIMARY KEY (source_workbook_id, source_sheet, source_row, source_column)
);

CREATE INDEX IF NOT EXISTS finance_account_balance_checkpoints_lookup
    ON finance_account_balance_checkpoints(account_id, checkpoint_date);
