-- A verified upstream collector credential can bind exactly one installation.
-- Retain its hash through token expiry so a revoked key cannot enroll a replacement.
CREATE TABLE collector_credentials (
 org TEXT NOT NULL, owner TEXT NOT NULL, token_hash TEXT NOT NULL CHECK(token_hash ~ '^[0-9a-f]{64}$'),
 device_id UUID NOT NULL REFERENCES registered_devices(id), expires_at TIMESTAMPTZ NOT NULL,
 PRIMARY KEY(org,owner,token_hash)
);
CREATE INDEX collector_credential_expiry ON collector_credentials(expires_at);
ALTER TABLE collector_credentials ENABLE ROW LEVEL SECURITY;
-- This internal table exposes no client route, token text or context. Owner scope
-- stays visible after service narrowing to prevent hidden-token re-enrollment.
CREATE POLICY collector_credential_owner ON collector_credentials USING
 (org=current_setting('wk.org',true) AND owner=current_setting('wk.user',true)) WITH CHECK
 (org=current_setting('wk.org',true) AND owner=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM registered_devices d WHERE d.id=collector_credentials.device_id AND d.org=collector_credentials.org AND d.owner=collector_credentials.owner));
