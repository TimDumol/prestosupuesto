-- Phase B canonical-finance import schema.
-- Monetary inputs and source-calculated values use decimal TEXT. Export parity
-- work will settle accepted scales and arithmetic/rounding before live writes.
CREATE TABLE IF NOT EXISTS finance_accounts (
    id TEXT PRIMARY KEY NOT NULL,
    source_name TEXT NOT NULL UNIQUE,
    account_type TEXT,
    currency TEXT CHECK (currency IS NULL OR currency IN ('EUR', 'PHP', 'USD')),
    is_real INTEGER NOT NULL CHECK (is_real IN (0, 1)),
    is_active INTEGER NOT NULL CHECK (is_active IN (0, 1)),
    current_native_balance TEXT,
    actual_native_balance TEXT,
    reconciliation_native TEXT,
    current_eur_balance TEXT,
    actual_eur_balance TEXT,
    source_row INTEGER NOT NULL,
    source_row_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS finance_categories (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    is_real INTEGER NOT NULL CHECK (is_real IN (0, 1)),
    source_row INTEGER NOT NULL,
    source_row_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS category_budget_periods (
    category_id TEXT NOT NULL REFERENCES finance_categories(id),
    effective_start_month TEXT NOT NULL CHECK (effective_start_month GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'),
    base_monthly_eur TEXT,
    source_row INTEGER NOT NULL,
    source_cell_json TEXT NOT NULL,
    PRIMARY KEY (category_id, effective_start_month)
);

CREATE TABLE IF NOT EXISTS finance_transactions (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('expense', 'income', 'transfer', 'reallocate', 'reconciliation')),
    effective_date TEXT NOT NULL CHECK (effective_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    description TEXT NOT NULL,
    amount_decimal TEXT NOT NULL,
    input_currency TEXT CHECK (input_currency IS NULL OR input_currency IN ('EUR', 'PHP', 'USD')),
    to_amount_decimal TEXT,
    to_currency TEXT CHECK (to_currency IS NULL OR to_currency IN ('EUR', 'PHP', 'USD')),
    from_category_id TEXT REFERENCES finance_categories(id),
    to_category_id TEXT REFERENCES finance_categories(id),
    from_account_id TEXT REFERENCES finance_accounts(id),
    to_account_id TEXT REFERENCES finance_accounts(id),
    currency_is_override INTEGER NOT NULL CHECK (currency_is_override IN (0, 1)),
    to_amount_is_override INTEGER NOT NULL CHECK (to_amount_is_override IN (0, 1)),
    created_at TEXT,
    created_by TEXT,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    source_workbook_id TEXT NOT NULL,
    source_sheet TEXT NOT NULL,
    source_row INTEGER NOT NULL,
    original_row_fingerprint TEXT NOT NULL,
    source_row_json TEXT NOT NULL,
    UNIQUE (source_workbook_id, source_sheet, source_row)
);

CREATE INDEX IF NOT EXISTS finance_transactions_effective_date
    ON finance_transactions(effective_date, id);
CREATE INDEX IF NOT EXISTS finance_transactions_from_account
    ON finance_transactions(from_account_id, effective_date);
CREATE INDEX IF NOT EXISTS finance_transactions_to_account
    ON finance_transactions(to_account_id, effective_date);
CREATE INDEX IF NOT EXISTS finance_transactions_from_category
    ON finance_transactions(from_category_id, effective_date);
CREATE INDEX IF NOT EXISTS finance_transactions_to_category
    ON finance_transactions(to_category_id, effective_date);

CREATE TABLE IF NOT EXISTS finance_operations (
    operation_id TEXT PRIMARY KEY NOT NULL,
    actor_subject TEXT NOT NULL,
    payload_fingerprint TEXT NOT NULL,
    affected_transaction_ids_json TEXT NOT NULL,
    response_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS finance_transaction_revisions (
    transaction_id TEXT NOT NULL REFERENCES finance_transactions(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision > 0),
    operation_id TEXT REFERENCES finance_operations(operation_id),
    snapshot_json TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (transaction_id, revision)
);

CREATE TABLE IF NOT EXISTS transaction_valuations (
    transaction_id TEXT NOT NULL REFERENCES finance_transactions(id) ON DELETE CASCADE,
    side TEXT NOT NULL CHECK (side IN ('from', 'to')),
    account_currency TEXT CHECK (account_currency IS NULL OR account_currency IN ('EUR', 'PHP', 'USD')),
    exchange_to_eur TEXT,
    exchange_to_native TEXT,
    amount_native TEXT,
    amount_eur TEXT,
    effective_date TEXT NOT NULL,
    valuation_source TEXT NOT NULL,
    rate_set_version TEXT,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    source_cells_json TEXT NOT NULL,
    PRIMARY KEY (transaction_id, side, revision)
);
CREATE INDEX IF NOT EXISTS transaction_valuations_current
    ON transaction_valuations(transaction_id, side, revision DESC);

CREATE TABLE IF NOT EXISTS transaction_valuation_source_cells (
    transaction_id TEXT NOT NULL REFERENCES finance_transactions(id) ON DELETE CASCADE,
    source_column TEXT NOT NULL CHECK (source_column IN ('J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q')),
    source_state TEXT NOT NULL CHECK (source_state IN ('formula', 'literal', 'blank', 'error')),
    source_formula TEXT,
    evaluated_value_json TEXT,
    formatted_value TEXT,
    PRIMARY KEY (transaction_id, source_column)
);

CREATE TABLE IF NOT EXISTS budget_month_snapshots (
    category_id TEXT NOT NULL REFERENCES finance_categories(id),
    month TEXT NOT NULL CHECK (month GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'),
    expense_eur TEXT,
    remaining_eur TEXT,
    reallocation_impact_eur TEXT,
    source_expense_state TEXT NOT NULL CHECK (source_expense_state IN ('formula', 'literal', 'blank', 'error')),
    source_remaining_state TEXT NOT NULL CHECK (source_remaining_state IN ('formula', 'literal', 'blank', 'error')),
    source_row INTEGER NOT NULL,
    source_cells_json TEXT NOT NULL,
    PRIMARY KEY (category_id, month)
);

CREATE TABLE IF NOT EXISTS budget_opening_state (
    category_id TEXT NOT NULL REFERENCES finance_categories(id),
    effective_before_month TEXT NOT NULL CHECK (effective_before_month GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'),
    remaining_eur TEXT NOT NULL,
    verified_by TEXT NOT NULL,
    verified_at TEXT NOT NULL,
    source_json TEXT NOT NULL,
    PRIMARY KEY (category_id, effective_before_month)
);

CREATE TABLE IF NOT EXISTS finance_exchange_rate_source_rows (
    source_workbook_id TEXT NOT NULL,
    source_sheet TEXT NOT NULL,
    source_row INTEGER NOT NULL,
    original_row_fingerprint TEXT NOT NULL,
    source_row_json TEXT NOT NULL,
    PRIMARY KEY (source_workbook_id, source_sheet, source_row)
);

CREATE TABLE IF NOT EXISTS exchange_rates (
    id TEXT PRIMARY KEY NOT NULL,
    rate_date TEXT,
    base_currency TEXT CHECK (base_currency IS NULL OR base_currency IN ('EUR', 'PHP', 'USD')),
    quote_currency TEXT CHECK (quote_currency IS NULL OR quote_currency IN ('EUR', 'PHP', 'USD')),
    rate_decimal TEXT,
    provider TEXT,
    source_pair TEXT,
    source_column TEXT NOT NULL,
    source_formula TEXT,
    source_value_json TEXT NOT NULL,
    source_workbook_id TEXT NOT NULL,
    source_sheet TEXT NOT NULL,
    source_row INTEGER NOT NULL,
    original_row_fingerprint TEXT NOT NULL,
    UNIQUE (source_workbook_id, source_sheet, source_row, source_column),
    CHECK (base_currency IS NULL OR quote_currency IS NULL OR base_currency <> quote_currency)
);
CREATE INDEX IF NOT EXISTS exchange_rates_pair_date
    ON exchange_rates(base_currency, quote_currency, rate_date);

CREATE TABLE IF NOT EXISTS finance_account_actuals (
    account_id TEXT PRIMARY KEY NOT NULL REFERENCES finance_accounts(id),
    actual_native_balance TEXT,
    source_reconciliation_native TEXT,
    source_row INTEGER NOT NULL,
    source_row_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS reconciliation_batches (
    id TEXT PRIMARY KEY NOT NULL,
    operation_id TEXT NOT NULL UNIQUE REFERENCES finance_operations(operation_id),
    effective_date TEXT NOT NULL,
    actor_subject TEXT NOT NULL,
    submitted_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS reconciliation_items (
    batch_id TEXT NOT NULL REFERENCES reconciliation_batches(id) ON DELETE CASCADE,
    account_id TEXT NOT NULL REFERENCES finance_accounts(id),
    transaction_id TEXT REFERENCES finance_transactions(id),
    actual_native_balance TEXT,
    variance_native TEXT,
    account_revision INTEGER NOT NULL,
    PRIMARY KEY (batch_id, account_id)
);

CREATE TABLE IF NOT EXISTS finance_import_runs (
    id TEXT PRIMARY KEY NOT NULL,
    source_workbook_id TEXT NOT NULL,
    source_digest TEXT NOT NULL UNIQUE,
    manifest_json TEXT NOT NULL,
    counts_json TEXT NOT NULL,
    committed_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS finance_quarantined_rows (
    source_workbook_id TEXT NOT NULL,
    source_sheet TEXT NOT NULL,
    source_row INTEGER NOT NULL,
    reason_code TEXT NOT NULL,
    original_row_fingerprint TEXT NOT NULL,
    source_row_json TEXT NOT NULL,
    PRIMARY KEY (source_workbook_id, source_sheet, source_row)
);

CREATE TABLE IF NOT EXISTS finance_source_artifacts (
    source_workbook_id TEXT NOT NULL,
    artifact_key TEXT NOT NULL,
    source_json TEXT NOT NULL,
    PRIMARY KEY (source_workbook_id, artifact_key)
);
