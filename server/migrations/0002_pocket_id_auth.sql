ALTER TABLE oauth_states ADD COLUMN pocket_session_hash TEXT;
ALTER TABLE sessions ADD COLUMN pocket_session_hash TEXT;

CREATE TABLE IF NOT EXISTS pocket_id_states (
    state TEXT PRIMARY KEY NOT NULL,
    code_verifier TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS pocket_sessions (
    id_hash TEXT PRIMARY KEY NOT NULL,
    subject TEXT NOT NULL,
    username TEXT NOT NULL,
    email TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS pocket_sessions_expires_at ON pocket_sessions(expires_at);
