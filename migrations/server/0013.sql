-- Serialize bootstrap and authenticated administrative mutations before row locks.
-- CREATE OR REPLACE preserves existing restricted EXECUTE grants.
CREATE OR REPLACE FUNCTION wk_admin_ceiling(p TEXT) RETURNS TEXT[] LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE result TEXT[];
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(current_setting('wk.org',true),p,'administration')::text,0));
 SELECT r.services INTO result FROM public.product_admins r JOIN public.memberships m ON (m.org,m.product,m.user_id)=(r.org,r.product,r.user_id)
 WHERE r.org=current_setting('wk.org',true) AND r.product=p AND r.user_id=current_setting('wk.user',true) AND m.services @> r.services FOR SHARE OF r;
 IF result IS NULL THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 RETURN result;
END; $$;
CREATE OR REPLACE FUNCTION wk_admin_member(p TEXT,target TEXT,new_services TEXT[],review BOOLEAN,remove_member BOOLEAN) RETURNS VOID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ceiling TEXT[]; old_services TEXT[]; o TEXT:=current_setting('wk.org',true); who TEXT:=current_setting('wk.user',true);
BEGIN
 ceiling:=public.wk_admin_ceiling(p);
 IF target IS NULL OR length(target) NOT BETWEEN 1 AND 256 OR NOT EXISTS(SELECT 1 FROM public.identities WHERE org=o AND user_id=target) THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 IF NOT remove_member AND (new_services IS NULL OR cardinality(new_services) NOT BETWEEN 1 AND 50 OR NOT(new_services <@ ceiling) OR EXISTS(SELECT 1 FROM unnest(new_services) s WHERE s IS NULL OR s !~ '^[A-Za-z0-9_-]{1,128}$')) THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(o,target,'devices')::text,0));
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
REVOKE ALL ON FUNCTION wk_admin_ceiling(TEXT),wk_admin_member(TEXT,TEXT,TEXT[],BOOLEAN,BOOLEAN) FROM PUBLIC;
