ALTER TABLE pocket_id_states ADD COLUMN mobile_redirect_uri TEXT;

CREATE TABLE IF NOT EXISTS pocket_mobile_handoffs (
    code_hash TEXT PRIMARY KEY NOT NULL,
    subject TEXT NOT NULL,
    username TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS pocket_mobile_handoffs_expires_at
    ON pocket_mobile_handoffs(expires_at);
