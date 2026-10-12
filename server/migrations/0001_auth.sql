CREATE TABLE IF NOT EXISTS google_accounts (
    email TEXT PRIMARY KEY NOT NULL,
    refresh_token BLOB NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id_hash TEXT PRIMARY KEY NOT NULL,
    email TEXT NOT NULL REFERENCES google_accounts(email) ON DELETE CASCADE,
    access_token BLOB NOT NULL,
    access_expires_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS sessions_expires_at ON sessions(expires_at);

CREATE TABLE IF NOT EXISTS oauth_states (
    state TEXT PRIMARY KEY NOT NULL,
    code_verifier TEXT NOT NULL,
    force_consent INTEGER NOT NULL DEFAULT 0,
    expires_at INTEGER NOT NULL
);
