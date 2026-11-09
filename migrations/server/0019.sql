-- Personal MCP tokens: a long-lived, revocable credential that authenticates one user to the hosted
-- MCP endpoint of one product, and to nothing else. Only the SHA-256 of a 256-bit random secret is
-- stored. Like browser_sessions it is looked up before an Actor exists, so it has no row-level
-- policy: no client route returns this table, and every owner-facing query filters by org and user.
CREATE TABLE personal_tokens (
 id UUID PRIMARY KEY,
 token_hash TEXT NOT NULL UNIQUE CHECK(token_hash ~ '^[0-9a-f]{64}$'),
 org TEXT NOT NULL, user_id TEXT NOT NULL, product TEXT NOT NULL,
 label TEXT NOT NULL CHECK(char_length(label) BETWEEN 1 AND 64),
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL,
 last_used_at TIMESTAMPTZ,
 revoked_at TIMESTAMPTZ
);
CREATE INDEX personal_tokens_owner ON personal_tokens(org,user_id);
