CREATE TABLE registered_devices (
 id UUID PRIMARY KEY, org TEXT NOT NULL, product TEXT NOT NULL, owner TEXT NOT NULL,
 public_key TEXT NOT NULL CHECK(length(public_key)=64), approval_hash TEXT NOT NULL CHECK(length(approval_hash)=64),
 services TEXT[] NOT NULL CHECK(cardinality(services) BETWEEN 1 AND 20),
 settings JSONB NOT NULL DEFAULT '{"paused":false,"owner_sync":false,"activity":false}',
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0), created_at TIMESTAMPTZ NOT NULL DEFAULT now(), revoked_at TIMESTAMPTZ,
 UNIQUE(org,owner,public_key), FOREIGN KEY(org,product) REFERENCES products(org,id)
);
CREATE INDEX devices_owner ON registered_devices(org,product,owner,created_at DESC);
CREATE TABLE device_reports (
 device_id UUID PRIMARY KEY REFERENCES registered_devices(id), org TEXT NOT NULL, owner TEXT NOT NULL,
 body JSONB NOT NULL CHECK(octet_length(body::text)<=4096), reported_at TIMESTAMPTZ NOT NULL DEFAULT now(), expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '7 days'
);
CREATE TABLE device_nonces (
 device_id UUID NOT NULL REFERENCES registered_devices(id), org TEXT NOT NULL, owner TEXT NOT NULL,
 nonce UUID NOT NULL, expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '120 seconds', PRIMARY KEY(device_id,nonce)
);
CREATE INDEX device_nonce_expiry ON device_nonces(expires_at);
ALTER TABLE registered_devices ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_reports ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_nonces ENABLE ROW LEVEL SECURITY;
CREATE POLICY devices_owner ON registered_devices USING (org=current_setting('wk.org',true) AND owner=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=registered_devices.org AND m.product=registered_devices.product AND m.user_id=registered_devices.owner AND m.services @> registered_devices.services)) WITH CHECK (org=current_setting('wk.org',true) AND owner=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=registered_devices.org AND m.product=registered_devices.product AND m.user_id=registered_devices.owner AND m.services @> registered_devices.services));
CREATE POLICY reports_owner ON device_reports USING (org=current_setting('wk.org',true) AND owner=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM registered_devices d WHERE d.id=device_reports.device_id)) WITH CHECK (org=current_setting('wk.org',true) AND owner=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM registered_devices d WHERE d.id=device_reports.device_id AND d.org=device_reports.org AND d.owner=device_reports.owner));
CREATE POLICY nonces_owner ON device_nonces USING (org=current_setting('wk.org',true) AND owner=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM registered_devices d WHERE d.id=device_nonces.device_id)) WITH CHECK (org=current_setting('wk.org',true) AND owner=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM registered_devices d WHERE d.id=device_nonces.device_id AND d.org=device_nonces.org AND d.owner=device_nonces.owner));
-- Counts all owner registrations even after service scope narrows; no hidden count output.
CREATE FUNCTION wk_device_has_capacity(requested_org TEXT,requested_owner TEXT) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF current_setting('wk.org',true) IS DISTINCT FROM requested_org OR current_setting('wk.user',true) IS DISTINCT FROM requested_owner THEN RAISE EXCEPTION 'invalid owner' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(requested_org,requested_owner,'devices')::text,0));
 RETURN (SELECT count(*) FILTER (WHERE revoked_at IS NULL)<10 AND count(*)<1000 FROM public.registered_devices WHERE org=requested_org AND owner=requested_owner);
END; $$;
REVOKE ALL ON FUNCTION wk_device_has_capacity(TEXT,TEXT) FROM PUBLIC;
