-- Administrative authority is independent of review and confers no content access.
CREATE TABLE product_admins (
 org TEXT NOT NULL, product TEXT NOT NULL, user_id TEXT NOT NULL, services TEXT[] NOT NULL CHECK(cardinality(services) BETWEEN 1 AND 50),
 PRIMARY KEY(org,product,user_id), FOREIGN KEY(org,product,user_id) REFERENCES memberships(org,product,user_id) ON DELETE CASCADE
);
ALTER TABLE product_admins ENABLE ROW LEVEL SECURITY;
CREATE TABLE product_settings (
 org TEXT NOT NULL, product TEXT NOT NULL, body JSONB NOT NULL,
 PRIMARY KEY(org,product), FOREIGN KEY(org,product) REFERENCES products(org,id)
);
ALTER TABLE product_settings ENABLE ROW LEVEL SECURITY;
CREATE POLICY product_settings_member ON product_settings FOR SELECT USING
 (org=current_setting('wk.org',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=product_settings.org AND m.product=product_settings.product AND m.user_id=current_setting('wk.user',true)));
ALTER TABLE audit_events ADD COLUMN target_user TEXT CHECK(target_user IS NULL OR length(target_user) BETWEEN 1 AND 256);
CREATE INDEX audit_expiration ON audit_events(created_at);

-- Lock role through the request; confirm membership after device locks, before mutation.
CREATE FUNCTION wk_admin_ceiling(p TEXT) RETURNS TEXT[] LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE result TEXT[];
BEGIN
 SELECT r.services INTO result FROM public.product_admins r JOIN public.memberships m ON (m.org,m.product,m.user_id)=(r.org,r.product,r.user_id)
 WHERE r.org=current_setting('wk.org',true) AND r.product=p AND r.user_id=current_setting('wk.user',true) AND m.services @> r.services FOR SHARE OF r;
 IF result IS NULL THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 RETURN result;
END; $$;
CREATE FUNCTION wk_admin_confirm(p TEXT,ceiling TEXT[]) RETURNS VOID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 PERFORM 1 FROM public.memberships WHERE org=current_setting('wk.org',true) AND product=p AND user_id=current_setting('wk.user',true) AND services @> ceiling FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
END; $$;
CREATE FUNCTION wk_admin_members(p TEXT) RETURNS TABLE(user_id TEXT,services TEXT[],can_review BOOLEAN,policy_revision BIGINT) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ceiling TEXT[];
BEGIN
 ceiling:=public.wk_admin_ceiling(p); PERFORM public.wk_admin_confirm(p,ceiling);
 RETURN QUERY SELECT m.user_id,m.services,m.can_review,m.policy_revision FROM public.memberships m WHERE m.org=current_setting('wk.org',true) AND m.product=p AND m.services <@ ceiling ORDER BY m.user_id LIMIT 100;
END; $$;
CREATE FUNCTION wk_admin_member(p TEXT,target TEXT,new_services TEXT[],review BOOLEAN,remove_member BOOLEAN) RETURNS VOID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ceiling TEXT[]; old_services TEXT[]; o TEXT:=current_setting('wk.org',true); who TEXT:=current_setting('wk.user',true);
BEGIN
 ceiling:=public.wk_admin_ceiling(p);
 IF target IS NULL OR length(target) NOT BETWEEN 1 AND 256 OR NOT EXISTS(SELECT 1 FROM public.identities WHERE org=o AND user_id=target) THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 IF NOT remove_member AND (new_services IS NULL OR cardinality(new_services) NOT BETWEEN 1 AND 50 OR NOT(new_services <@ ceiling) OR EXISTS(SELECT 1 FROM unnest(new_services) s WHERE s IS NULL OR s !~ '^[A-Za-z0-9_-]{1,128}$')) THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 PERFORM 1 FROM public.registered_devices WHERE org=o AND product=p AND owner=target ORDER BY id FOR UPDATE;
 PERFORM public.wk_admin_confirm(p,ceiling);
 SELECT services INTO old_services FROM public.memberships WHERE org=o AND product=p AND user_id=target FOR UPDATE;
 IF old_services IS NOT NULL AND NOT(old_services <@ ceiling) THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 IF remove_member THEN
  UPDATE public.registered_devices SET revoked_at=now(),revision=revision+1 WHERE org=o AND product=p AND owner=target AND revoked_at IS NULL;
  DELETE FROM public.memberships WHERE org=o AND product=p AND user_id=target;
 ELSE
  UPDATE public.registered_devices SET revoked_at=now(),revision=revision+1 WHERE org=o AND product=p AND owner=target AND revoked_at IS NULL AND NOT(new_services @> services);
  INSERT INTO public.memberships(org,product,user_id,services,can_review) VALUES(o,p,target,new_services,review)
   ON CONFLICT(org,product,user_id) DO UPDATE SET services=EXCLUDED.services,can_review=EXCLUDED.can_review,policy_revision=memberships.policy_revision+1;
  DELETE FROM public.product_admins WHERE org=o AND product=p AND user_id=target AND NOT(new_services @> services);
 END IF;
 INSERT INTO public.audit_events(org,product,user_id,action,entity_id,target_user) VALUES(o,p,who,CASE WHEN remove_member THEN 'admin_member_remove' ELSE 'admin_member_put' END,'00000000-0000-0000-0000-000000000000',target);
END; $$;
CREATE FUNCTION wk_admin_device(p TEXT,installation UUID) RETURNS VOID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ceiling TEXT[]; target TEXT; scopes TEXT[]; o TEXT:=current_setting('wk.org',true);
BEGIN
 ceiling:=public.wk_admin_ceiling(p);
 SELECT owner,services INTO target,scopes FROM public.registered_devices WHERE id=installation AND org=o AND product=p FOR UPDATE;
 IF target IS NULL OR NOT(scopes <@ ceiling) THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 PERFORM public.wk_admin_confirm(p,ceiling);
 UPDATE public.registered_devices SET revoked_at=now(),revision=revision+1 WHERE id=installation AND revoked_at IS NULL;
 INSERT INTO public.audit_events(org,product,user_id,action,entity_id,target_user) VALUES(o,p,current_setting('wk.user',true),'admin_device_revoke',installation,target);
END; $$;

-- Only fixed, typed restrictions enter the internal policy table. No endpoint redirect.
CREATE FUNCTION wk_product_policy_apply(o TEXT,p TEXT,who TEXT,value JSONB,narrow BOOLEAN) RETURNS VOID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE old JSONB; private_days INTEGER; report_days INTEGER;
BEGIN
 IF jsonb_typeof(value) IS DISTINCT FROM 'object' OR (SELECT count(*) FROM jsonb_object_keys(value))<>4 OR NOT(value ?& ARRAY['private_days','report_days','allow_loopback','https_origins'])
 OR jsonb_typeof(value->'private_days') IS DISTINCT FROM 'number' OR jsonb_typeof(value->'report_days') IS DISTINCT FROM 'number' OR jsonb_typeof(value->'allow_loopback') IS DISTINCT FROM 'boolean' OR jsonb_typeof(value->'https_origins') IS DISTINCT FROM 'array' THEN RAISE EXCEPTION 'invalid' USING ERRCODE='22023'; END IF;
 private_days:=(value->>'private_days')::INTEGER; report_days:=(value->>'report_days')::INTEGER;
 IF private_days NOT BETWEEN 1 AND 30 OR report_days NOT BETWEEN 1 AND 7 OR jsonb_array_length(value->'https_origins')>16 OR EXISTS(SELECT 1 FROM jsonb_array_elements_text(value->'https_origins') s WHERE s !~ '^https://[A-Za-z0-9.-]+(:[0-9]{1,5})?$' OR length(s)>256) THEN RAISE EXCEPTION 'invalid' USING ERRCODE='22023'; END IF;
 SELECT body INTO old FROM public.product_settings WHERE org=o AND product=p FOR UPDATE;
 old:=COALESCE(old,'{"private_days":30,"report_days":7,"allow_loopback":true,"https_origins":[]}'::JSONB);
 IF narrow AND (private_days>(old->>'private_days')::INTEGER OR report_days>(old->>'report_days')::INTEGER OR ((value->>'allow_loopback')::BOOLEAN AND NOT((old->>'allow_loopback')::BOOLEAN)) OR NOT((old->'https_origins') @> (value->'https_origins'))) THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 INSERT INTO public.product_settings(org,product,body) VALUES(o,p,value) ON CONFLICT(org,product) DO UPDATE SET body=EXCLUDED.body;
 UPDATE public.memberships SET policy_revision=policy_revision+1 WHERE org=o AND product=p;
 UPDATE public.personal_summaries SET expires_at=LEAST(expires_at,created_at+private_days*interval '1 day') WHERE org=o AND product=p;
 UPDATE public.device_reports r SET expires_at=LEAST(r.expires_at,r.reported_at+report_days*interval '1 day') FROM public.registered_devices d WHERE d.id=r.device_id AND d.org=o AND d.product=p;
 INSERT INTO public.audit_events(org,product,user_id,action,entity_id) VALUES(o,p,who,CASE WHEN narrow THEN 'admin_policy_restrict' ELSE 'operator_policy_set' END,'00000000-0000-0000-0000-000000000000');
END; $$;
CREATE FUNCTION wk_admin_policy(p TEXT,value JSONB) RETURNS VOID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ceiling TEXT[]; o TEXT:=current_setting('wk.org',true);
BEGIN
 ceiling:=public.wk_admin_ceiling(p);
 PERFORM 1 FROM public.registered_devices WHERE org=o AND product=p ORDER BY id FOR UPDATE;
 PERFORM public.wk_admin_confirm(p,ceiling);
 PERFORM 1 FROM public.memberships WHERE org=o AND product=p ORDER BY user_id FOR UPDATE;
 IF EXISTS(SELECT 1 FROM public.memberships WHERE org=o AND product=p AND NOT(services <@ ceiling)) THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 PERFORM public.wk_product_policy_apply(o,p,current_setting('wk.user',true),value,true);
END; $$;
REVOKE ALL ON FUNCTION wk_admin_ceiling(TEXT),wk_admin_confirm(TEXT,TEXT[]),wk_admin_members(TEXT),wk_admin_member(TEXT,TEXT,TEXT[],BOOLEAN,BOOLEAN),wk_admin_device(TEXT,UUID),wk_product_policy_apply(TEXT,TEXT,TEXT,JSONB,BOOLEAN),wk_admin_policy(TEXT,JSONB) FROM PUBLIC;

CREATE FUNCTION wk_personal_policy_expiry() RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE days INTEGER;
BEGIN
 SELECT (body->>'private_days')::INTEGER INTO days FROM public.product_settings WHERE org=NEW.org AND product=NEW.product;
 NEW.expires_at:=LEAST(NEW.expires_at,NEW.created_at+COALESCE(days,30)*interval '1 day');
 IF TG_OP='UPDATE' THEN NEW.expires_at:=LEAST(NEW.expires_at,OLD.expires_at); END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER personal_policy_expiry BEFORE INSERT OR UPDATE ON personal_summaries FOR EACH ROW EXECUTE FUNCTION wk_personal_policy_expiry();
CREATE FUNCTION wk_report_policy_expiry() RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE days INTEGER;
BEGIN
 SELECT (p.body->>'report_days')::INTEGER INTO days FROM public.product_settings p JOIN public.registered_devices d ON (p.org,p.product)=(d.org,d.product) WHERE d.id=NEW.device_id;
 NEW.expires_at:=LEAST(NEW.expires_at,NEW.reported_at+COALESCE(days,7)*interval '1 day'); RETURN NEW;
END; $$;
CREATE TRIGGER report_policy_expiry BEFORE INSERT OR UPDATE ON device_reports FOR EACH ROW EXECUTE FUNCTION wk_report_policy_expiry();
REVOKE ALL ON FUNCTION wk_personal_policy_expiry(),wk_report_policy_expiry() FROM PUBLIC;
