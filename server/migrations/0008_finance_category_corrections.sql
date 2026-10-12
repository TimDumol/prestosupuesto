-- Audited category-only corrections to transactions in closed months.
-- They affect the current open budget month and never change account balances.
CREATE TABLE IF NOT EXISTS finance_category_corrections (
    operation_id TEXT PRIMARY KEY NOT NULL
        REFERENCES finance_operations(operation_id),
    original_transaction_id TEXT NOT NULL
        REFERENCES finance_transactions(id),
    original_effective_date TEXT NOT NULL
        CHECK (original_effective_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    applied_month TEXT NOT NULL
        CHECK (applied_month GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'),
    reason TEXT NOT NULL CHECK (length(trim(reason)) BETWEEN 1 AND 500),
    category_deltas_json TEXT NOT NULL,
    recorded_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS finance_category_corrections_original
    ON finance_category_corrections(original_transaction_id, recorded_at);

CREATE TABLE IF NOT EXISTS finance_category_correction_items (
    operation_id TEXT NOT NULL
        REFERENCES finance_category_corrections(operation_id) ON DELETE CASCADE,
    category_id TEXT NOT NULL REFERENCES finance_categories(id),
    expense_delta_eur TEXT NOT NULL,
    PRIMARY KEY (operation_id, category_id)
);

CREATE INDEX IF NOT EXISTS finance_category_correction_items_category
    ON finance_category_correction_items(category_id, operation_id);
