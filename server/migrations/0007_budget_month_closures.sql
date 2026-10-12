-- Explicit owner action closes and freezes a budget month. The close operation
-- and its digest-bound category snapshot are committed in one DB transaction.
CREATE TABLE IF NOT EXISTS budget_month_closures (
    month TEXT PRIMARY KEY NOT NULL
        CHECK (month GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'),
    operation_id TEXT NOT NULL UNIQUE
        REFERENCES finance_operations(operation_id),
    closed_at TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    snapshot_sha256 TEXT NOT NULL
        CHECK (length(snapshot_sha256) = 64 AND snapshot_sha256 NOT GLOB '*[^0-9a-f]*')
);

CREATE TRIGGER IF NOT EXISTS budget_month_closures_no_update
BEFORE UPDATE ON budget_month_closures
BEGIN
    SELECT RAISE(ABORT, 'budget month close is immutable');
END;

CREATE TRIGGER IF NOT EXISTS budget_month_closures_no_delete
BEFORE DELETE ON budget_month_closures
BEGIN
    SELECT RAISE(ABORT, 'budget month close is immutable');
END;

CREATE TRIGGER IF NOT EXISTS budget_month_snapshots_closed_insert
BEFORE INSERT ON budget_month_snapshots
WHEN EXISTS (
    SELECT 1 FROM budget_month_closures WHERE month = NEW.month
)
BEGIN
    SELECT RAISE(ABORT, 'budget month is closed');
END;

CREATE TRIGGER IF NOT EXISTS budget_month_snapshots_closed_update
BEFORE UPDATE ON budget_month_snapshots
WHEN EXISTS (
    SELECT 1 FROM budget_month_closures
    WHERE month = OLD.month OR month = NEW.month
)
BEGIN
    SELECT RAISE(ABORT, 'budget month is closed');
END;

CREATE TRIGGER IF NOT EXISTS budget_month_snapshots_closed_delete
BEFORE DELETE ON budget_month_snapshots
WHEN EXISTS (
    SELECT 1 FROM budget_month_closures WHERE month = OLD.month
)
BEGIN
    SELECT RAISE(ABORT, 'budget month is closed');
END;
