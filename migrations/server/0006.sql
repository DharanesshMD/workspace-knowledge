-- High entropy hashes authenticate browser sessions before an Actor is known.
-- No client-facing API exposes this table; grant it only to the server role.
CREATE TABLE browser_sessions (
 session_hash TEXT PRIMARY KEY CHECK(session_hash ~ '^[0-9a-f]{64}$'),
 issuer TEXT NOT NULL, subject TEXT NOT NULL,
 csrf_hash TEXT NOT NULL CHECK(csrf_hash ~ '^[0-9a-f]{64}$'),
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 last_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '12 hours',
 FOREIGN KEY(issuer,subject) REFERENCES identities(issuer,subject) ON DELETE CASCADE
);
CREATE INDEX browser_sessions_expiry ON browser_sessions(expires_at);
CREATE INDEX browser_sessions_identity ON browser_sessions(issuer,subject);
