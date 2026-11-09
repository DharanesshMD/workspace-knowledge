-- Admin-created, token-only developer accounts. A product admin (within the services ceiling that
-- wk_admin_ceiling enforces) can create a developer who never signs in: the account is an identity under
-- the reserved issuer 'wk-admin-created' (not a URL, so no verified identity-provider token can carry it)
-- plus an ordinary membership. The restricted application role cannot write identities, so these run as
-- SECURITY DEFINER functions, like the other administrative functions, and are granted to it explicitly.

-- Is the caller an admin of this product (with a ceiling their membership still covers)?
CREATE FUNCTION wk_is_admin(p TEXT) RETURNS BOOLEAN LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT EXISTS(SELECT 1 FROM public.product_admins r JOIN public.memberships m ON (m.org,m.product,m.user_id)=(r.org,r.product,r.user_id)
  WHERE r.org=current_setting('wk.org',true) AND r.product=p AND r.user_id=current_setting('wk.user',true) AND m.services @> r.services);
$$;

-- The admin's ceiling and every member inside it. token_only is true only for accounts whose every
-- identity is admin-created; a signed-in developer is never token_only.
CREATE FUNCTION wk_admin_users(p TEXT) RETURNS JSONB LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ceiling TEXT[]; o TEXT:=current_setting('wk.org',true);
BEGIN
 ceiling:=public.wk_admin_ceiling(p); PERFORM public.wk_admin_confirm(p,ceiling);
 RETURN jsonb_build_object('services',to_jsonb(ceiling),'users',COALESCE((
  SELECT jsonb_agg(jsonb_build_object('user',m.user_id,'services',m.services,'can_review',m.can_review,
   'token_only',(EXISTS(SELECT 1 FROM public.identities i WHERE i.org=o AND i.user_id=m.user_id AND i.issuer='wk-admin-created')
    AND NOT EXISTS(SELECT 1 FROM public.identities i WHERE i.org=o AND i.user_id=m.user_id AND i.issuer<>'wk-admin-created'))) ORDER BY m.user_id)
  FROM (SELECT user_id,services,can_review FROM public.memberships WHERE org=o AND product=p AND services <@ ceiling ORDER BY user_id LIMIT 100) m),'[]'::jsonb));
END; $$;

-- May the caller issue tokens for this user? Only for a token-only account inside the ceiling.
CREATE FUNCTION wk_admin_token_target(p TEXT,target TEXT) RETURNS VOID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ceiling TEXT[]; o TEXT:=current_setting('wk.org',true);
BEGIN
 ceiling:=public.wk_admin_ceiling(p); PERFORM public.wk_admin_confirm(p,ceiling);
 IF target IS NULL OR NOT EXISTS(SELECT 1 FROM public.memberships m WHERE m.org=o AND m.product=p AND m.user_id=target AND m.services <@ ceiling)
  OR NOT EXISTS(SELECT 1 FROM public.identities i WHERE i.org=o AND i.user_id=target AND i.issuer='wk-admin-created')
  OR EXISTS(SELECT 1 FROM public.identities i WHERE i.org=o AND i.user_id=target AND i.issuer<>'wk-admin-created') THEN
  RAISE EXCEPTION 'denied' USING ERRCODE='42501';
 END IF;
END; $$;

-- Create the account and its membership. A name that already exists as any identity or any membership in
-- the organization is refused, so this can never attach to (or take over) a signed-in developer.
CREATE FUNCTION wk_admin_create_user(p TEXT,target TEXT,new_services TEXT[],review BOOLEAN) RETURNS VOID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ceiling TEXT[]; o TEXT:=current_setting('wk.org',true); who TEXT:=current_setting('wk.user',true);
BEGIN
 ceiling:=public.wk_admin_ceiling(p);
 IF target IS NULL OR target !~ '^[a-z][a-z0-9._-]{0,31}$' THEN RAISE EXCEPTION 'invalid' USING ERRCODE='22023'; END IF;
 IF new_services IS NULL OR cardinality(new_services) NOT BETWEEN 1 AND 50 OR NOT(new_services <@ ceiling)
  OR EXISTS(SELECT 1 FROM unnest(new_services) s WHERE s IS NULL OR s !~ '^[A-Za-z0-9_-]{1,128}$') THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(o,target,'identity')::text,0));
 PERFORM public.wk_admin_confirm(p,ceiling);
 IF EXISTS(SELECT 1 FROM public.identities WHERE org=o AND user_id=target) OR EXISTS(SELECT 1 FROM public.memberships WHERE org=o AND user_id=target) THEN
  RAISE EXCEPTION 'conflict' USING ERRCODE='23505';
 END IF;
 INSERT INTO public.identities(issuer,subject,org,user_id) VALUES('wk-admin-created',gen_random_uuid()::text,o,target);
 INSERT INTO public.memberships(org,product,user_id,services,can_review) VALUES(o,p,target,new_services,review);
 INSERT INTO public.audit_events(org,product,user_id,action,entity_id,target_user) VALUES(o,p,who,'admin_user_create','00000000-0000-0000-0000-000000000000',target);
END; $$;

-- After the membership is removed: forget a token-only account that belongs to nothing any more, so the
-- name can be reused, and drop every token it ever had so a new account of that name inherits none.
CREATE FUNCTION wk_admin_delete_user(p TEXT,target TEXT) RETURNS VOID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ceiling TEXT[]; o TEXT:=current_setting('wk.org',true);
BEGIN
 ceiling:=public.wk_admin_ceiling(p); PERFORM public.wk_admin_confirm(p,ceiling);
 IF target IS NULL OR NOT EXISTS(SELECT 1 FROM public.identities i WHERE i.org=o AND i.user_id=target AND i.issuer='wk-admin-created')
  OR EXISTS(SELECT 1 FROM public.identities i WHERE i.org=o AND i.user_id=target AND i.issuer<>'wk-admin-created') THEN
  RETURN;
 END IF;
 IF EXISTS(SELECT 1 FROM public.memberships WHERE org=o AND user_id=target) THEN RETURN; END IF;
 DELETE FROM public.personal_tokens WHERE org=o AND user_id=target;
 DELETE FROM public.identities WHERE org=o AND user_id=target AND issuer='wk-admin-created';
 INSERT INTO public.audit_events(org,product,user_id,action,entity_id,target_user) VALUES(o,p,current_setting('wk.user',true),'admin_user_delete','00000000-0000-0000-0000-000000000000',target);
END; $$;

REVOKE ALL ON FUNCTION wk_is_admin(TEXT),wk_admin_users(TEXT),wk_admin_token_target(TEXT,TEXT),wk_admin_create_user(TEXT,TEXT,TEXT[],BOOLEAN),wk_admin_delete_user(TEXT,TEXT) FROM PUBLIC;
