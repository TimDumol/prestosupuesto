-- Operational metadata is deliberately excluded from finance recovery archives.
CREATE TABLE push_devices (
    installation_id TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    session_hash TEXT NOT NULL,
    token TEXT NOT NULL UNIQUE,
    updated_at INTEGER NOT NULL
);
CREATE TABLE push_outbox (
    event_id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL,
    transaction_id TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    recipient_subject TEXT NOT NULL,
    session_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    next_attempt_at INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    UNIQUE(operation_id, installation_id)
);
CREATE INDEX push_outbox_due ON push_outbox(next_attempt_at);
