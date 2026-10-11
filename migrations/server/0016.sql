-- Briefs are private, temporary authorization state, never shared knowledge.
CREATE TABLE code_preflights (
 id UUID PRIMARY KEY,org TEXT NOT NULL,product TEXT NOT NULL,owner TEXT NOT NULL,
 device_id UUID NOT NULL REFERENCES registered_devices(id),binding_id UUID NOT NULL REFERENCES workspace_bindings(id),
 security_epoch BIGINT NOT NULL,services TEXT[] NOT NULL CHECK(cardinality(services) BETWEEN 1 AND 50),
 policy_revision BIGINT NOT NULL,repository_id UUID NOT NULL,repository_revision BIGINT NOT NULL,
 operation_handle UUID NOT NULL,request JSONB NOT NULL CHECK(octet_length(request::text)<=65536),
 source_versions JSONB NOT NULL CHECK(octet_length(source_versions::text)<=65536),
 brief JSONB NOT NULL CHECK(octet_length(brief::text)<=32768),
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),expires_at TIMESTAMPTZ NOT NULL,
 cleanup_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '24 hours',
 acknowledged_at TIMESTAMPTZ,consumed_at TIMESTAMPTZ,
 UNIQUE(org,product,owner,device_id,operation_handle),
 FOREIGN KEY(org,product,repository_id) REFERENCES code_repositories(org,product,id),
 CHECK(expires_at<=created_at+interval '5 minutes'),CHECK(cleanup_at<=created_at+interval '24 hours')
);
CREATE INDEX code_preflight_expiration ON code_preflights(cleanup_at);
CREATE INDEX code_preflight_owner ON code_preflights(org,product,owner,expires_at);
ALTER TABLE code_preflights ENABLE ROW LEVEL SECURITY;
CREATE POLICY code_preflight_owner ON code_preflights USING (
 org=current_setting('wk.org',true) AND owner=current_setting('wk.user',true)
 AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_preflights.org AND m.product=code_preflights.product AND m.user_id=code_preflights.owner AND m.services @> code_preflights.services)
) WITH CHECK (
 org=current_setting('wk.org',true) AND owner=current_setting('wk.user',true)
 AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_preflights.org AND m.product=code_preflights.product AND m.user_id=code_preflights.owner AND m.services @> code_preflights.services)
);
-- Epoch locking does not authorize the runtime to reset the restore cutoff.
ALTER TABLE collector_security_epoch ENABLE ROW LEVEL SECURITY;
CREATE POLICY collector_epoch_read ON collector_security_epoch FOR SELECT USING (true);
CREATE POLICY collector_epoch_lock ON collector_security_epoch FOR UPDATE USING (true) WITH CHECK (false);
CREATE FUNCTION wk_preflight_has_capacity(o TEXT,p TEXT,who TEXT) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF current_setting('wk.org',true) IS DISTINCT FROM o OR current_setting('wk.user',true) IS DISTINCT FROM who THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 RETURN (SELECT count(*)<2000 AND count(*) FILTER (WHERE expires_at>clock_timestamp() AND consumed_at IS NULL)<100 FROM public.code_preflights WHERE org=o AND product=p AND owner=who);
END; $$;
REVOKE ALL ON FUNCTION wk_preflight_has_capacity(TEXT,TEXT,TEXT) FROM PUBLIC;

-- Withdrawal remains possible after the owner's service membership changes.
-- Other users still need current approved evidence and all profile services.
CREATE POLICY code_profile_owned_metadata ON code_profiles FOR SELECT USING (
 org=current_setting('wk.org',true) AND user_id=current_setting('wk.user',true)
);
CREATE POLICY code_profile_withdraw ON code_profiles FOR UPDATE USING (
 org=current_setting('wk.org',true) AND user_id=current_setting('wk.user',true)
) WITH CHECK (
 org=current_setting('wk.org',true) AND user_id=current_setting('wk.user',true)
 AND display_name IS NULL AND contact_url IS NULL AND revoked
);
