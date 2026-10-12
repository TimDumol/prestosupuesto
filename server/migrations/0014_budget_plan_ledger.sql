-- Plan funding is independent of cash. Existing category facts retain their
-- meaning at activation; subsequent salary receipts settle expectations.
CREATE TABLE budget_ledger_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    start_month TEXT NOT NULL,
    opening_unallocated_eur TEXT NOT NULL,
    provenance_json TEXT NOT NULL,
    activated_at TEXT NOT NULL
);
CREATE TABLE budget_plan_income (
    month TEXT PRIMARY KEY,
    amount TEXT NOT NULL,
    currency TEXT NOT NULL,
    amount_eur TEXT NOT NULL,
    arrival_day INTEGER NOT NULL,
    completed_month TEXT,
    completion_shortfall_eur TEXT,
    completion_operation_id TEXT REFERENCES finance_operations(operation_id)
);
CREATE TABLE budget_income_receipts (
    transaction_id TEXT PRIMARY KEY,
    treatment TEXT NOT NULL CHECK(treatment IN ('salary','unexpected','refund','legacy')),
    expected_month TEXT REFERENCES budget_plan_income(month)
);
CREATE TABLE budget_ledger_movements (
    id INTEGER PRIMARY KEY,
    month TEXT NOT NULL,
    category_id TEXT NOT NULL REFERENCES finance_categories(id),
    kind TEXT NOT NULL,
    source_key TEXT NOT NULL,
    amount_eur TEXT NOT NULL,
    operation_id TEXT REFERENCES finance_operations(operation_id),
    recorded_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX budget_ledger_movements_source ON budget_ledger_movements(source_key,category_id,kind);
CREATE INDEX budget_ledger_movements_month ON budget_ledger_movements(month,category_id);
CREATE TRIGGER budget_ledger_movements_no_update BEFORE UPDATE ON budget_ledger_movements
BEGIN SELECT RAISE(ABORT,'budget ledger movement is immutable'); END;
CREATE TRIGGER budget_ledger_movements_no_delete BEFORE DELETE ON budget_ledger_movements
BEGIN SELECT RAISE(ABORT,'budget ledger movement is immutable'); END;
CREATE TRIGGER budget_ledger_movements_closed BEFORE INSERT ON budget_ledger_movements
WHEN EXISTS(SELECT 1 FROM budget_month_closures WHERE month=NEW.month)
BEGIN SELECT RAISE(ABORT,'budget month is closed'); END;
CREATE TABLE budget_coverage_closures (
    month TEXT PRIMARY KEY REFERENCES budget_month_closures(month),
    snapshot_json TEXT NOT NULL,
    snapshot_sha256 TEXT NOT NULL
);
CREATE TRIGGER budget_coverage_closures_no_update BEFORE UPDATE ON budget_coverage_closures
BEGIN SELECT RAISE(ABORT,'budget coverage close is immutable'); END;
CREATE TRIGGER budget_coverage_closures_no_delete BEFORE DELETE ON budget_coverage_closures
BEGIN SELECT RAISE(ABORT,'budget coverage close is immutable'); END;
