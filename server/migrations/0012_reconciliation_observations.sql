-- Preserve the current observation and the activity used to reconstruct closing balances.
ALTER TABLE reconciliation_batches ADD COLUMN observed_at TEXT;
ALTER TABLE reconciliation_batches ADD COLUMN observed_date TEXT;
ALTER TABLE reconciliation_items ADD COLUMN observed_native_balance TEXT;
ALTER TABLE reconciliation_items ADD COLUMN post_month_activity_native TEXT;
ALTER TABLE reconciliation_items ADD COLUMN included_transactions_json TEXT;
