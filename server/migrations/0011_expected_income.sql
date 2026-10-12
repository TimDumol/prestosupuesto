-- Recurring expected income, effective from a budget month onward.
CREATE TABLE IF NOT EXISTS budget_income_expectations (
    effective_start_month TEXT PRIMARY KEY NOT NULL,
    amount TEXT NOT NULL,
    currency TEXT NOT NULL CHECK (currency IN ('EUR', 'USD', 'PHP')),
    arrival_day INTEGER NOT NULL CHECK (arrival_day BETWEEN 1 AND 31)
);
