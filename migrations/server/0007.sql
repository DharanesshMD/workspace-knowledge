-- Owner-only hosted summaries. These never participate in shared retrieval.
CREATE TABLE personal_summaries (
 id UUID PRIMARY KEY, org TEXT NOT NULL, product TEXT NOT NULL, owner TEXT NOT NULL,
 idempotency_key TEXT NOT NULL CHECK(length(idempotency_key) BETWEEN 1 AND 128),
 services TEXT[] NOT NULL, body JSONB, provenance TEXT NOT NULL CHECK(provenance IN ('manual','captured')),
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0), submitted_card UUID,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(), expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '30 days',
 deleted BOOLEAN NOT NULL DEFAULT FALSE,
 UNIQUE(org,product,owner,idempotency_key),
 FOREIGN KEY(org,product) REFERENCES products(org,id),
 FOREIGN KEY(org,product,submitted_card) REFERENCES cards(org,product,id),
 CHECK(body IS NULL OR octet_length(body::text)<=65536),
 CHECK(NOT deleted OR body IS NULL)
);
CREATE INDEX personal_owner_page ON personal_summaries(org,product,owner,created_at DESC,id DESC);
CREATE INDEX personal_expiration ON personal_summaries(expires_at) WHERE body IS NOT NULL;
ALTER TABLE personal_summaries ENABLE ROW LEVEL SECURITY;
CREATE POLICY personal_owner ON personal_summaries USING
 (org=current_setting('wk.org',true) AND owner=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=personal_summaries.org AND m.product=personal_summaries.product AND m.user_id=personal_summaries.owner AND m.services @> personal_summaries.services))
 WITH CHECK
 (org=current_setting('wk.org',true) AND owner=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=personal_summaries.org AND m.product=personal_summaries.product AND m.user_id=personal_summaries.owner AND m.services @> personal_summaries.services));

-- Capacity includes the owner's old service scopes. Return only a boolean,
-- never another owner's count or content. Schema creation is denied to wk_app.
CREATE FUNCTION wk_personal_has_capacity(requested_org TEXT,requested_product TEXT,requested_owner TEXT)
RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF current_setting('wk.org',true) IS DISTINCT FROM requested_org OR current_setting('wk.user',true) IS DISTINCT FROM requested_owner THEN
  RAISE EXCEPTION 'invalid owner' USING ERRCODE='42501';
 END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(requested_org,requested_product,requested_owner,'personal')::text,0));
 RETURN (SELECT count(*)<2000 FROM public.personal_summaries WHERE org=requested_org AND product=requested_product AND owner=requested_owner AND NOT deleted AND expires_at>now());
END;
$$;
REVOKE ALL ON FUNCTION wk_personal_has_capacity(TEXT,TEXT,TEXT) FROM PUBLIC;
