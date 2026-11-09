-- Principal names are permanent within an organization. Identity deletion used to permit a new
-- token-only account to adopt retained private summaries, drafts, activity and code provenance.
-- Reserve every historical principal, including accounts deleted before this migration. Preserve
-- the reservation independently of content retention and the private-deletion journal.
CREATE TABLE principal_reservations (
 org TEXT NOT NULL, user_id TEXT NOT NULL, PRIMARY KEY(org,user_id)
);
REVOKE ALL ON principal_reservations FROM PUBLIC;
INSERT INTO public.principal_reservations(org,user_id)
SELECT org,user_id FROM (
 SELECT org,user_id AS user_id FROM public.identities
 UNION ALL
 SELECT org,user_id AS user_id FROM public.memberships
 UNION ALL
 SELECT org,author AS user_id FROM public.cards
 UNION ALL
 SELECT org,changed_by AS user_id FROM public.card_revisions
 UNION ALL
 SELECT org,user_id AS user_id FROM public.feedback
 UNION ALL
 SELECT org,user_id AS user_id FROM public.audit_events
 UNION ALL
 SELECT org,target_user AS user_id FROM public.audit_events
 UNION ALL
 SELECT org,user_id AS user_id FROM public.activity
 UNION ALL
 SELECT org,user_id AS user_id FROM public.workspace_bindings
 UNION ALL
 SELECT org,owner AS user_id FROM public.personal_summaries
 UNION ALL
 SELECT org,owner AS user_id FROM public.personal_deletions
 UNION ALL
 SELECT org,owner AS user_id FROM public.registered_devices
 UNION ALL
 SELECT org,owner AS user_id FROM public.device_reports
 UNION ALL
 SELECT org,owner AS user_id FROM public.device_nonces
 UNION ALL
 SELECT org,owner AS user_id FROM public.collector_credentials
 UNION ALL
 SELECT org,user_id AS user_id FROM public.product_admins
 UNION ALL
 SELECT org,user_id AS user_id FROM public.personal_tokens
 UNION ALL
 SELECT org,user_id AS user_id FROM public.code_profiles
 UNION ALL
 SELECT org,user_id AS user_id FROM public.code_mappings
 UNION ALL
 SELECT org,reviewer AS user_id FROM public.code_mappings
 UNION ALL
 SELECT org,user_id AS user_id FROM public.code_maintainers
 UNION ALL
 SELECT org,reviewer AS user_id FROM public.code_maintainers
 UNION ALL
 SELECT org,owner AS user_id FROM public.code_preflights
 UNION ALL
 SELECT org,owner AS user_id FROM public.code_contributions
 UNION ALL
 SELECT org,submitted_by AS user_id FROM public.code_contributions
 UNION ALL
 SELECT org,confirmed_by AS user_id FROM public.code_contributions
 UNION ALL
 SELECT org,reviewed_by AS user_id FROM public.code_contributions
 UNION ALL
 SELECT org,owner AS user_id FROM public.code_rationale_anchors
 UNION ALL
 SELECT org,submitted_by AS user_id FROM public.code_rationale_anchors
 UNION ALL
 SELECT org,confirmed_by AS user_id FROM public.code_rationale_anchors
 UNION ALL
 SELECT org,reviewed_by AS user_id FROM public.code_rationale_anchors
) historical WHERE user_id IS NOT NULL AND user_id<>''
ON CONFLICT DO NOTHING;

-- CREATE OR REPLACE preserves the restricted application's existing EXECUTE grants.
CREATE OR REPLACE FUNCTION wk_admin_create_user(p TEXT,target TEXT,new_services TEXT[],review BOOLEAN) RETURNS VOID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ceiling TEXT[]; o TEXT:=current_setting('wk.org',true); who TEXT:=current_setting('wk.user',true);
BEGIN
 ceiling:=public.wk_admin_ceiling(p);
 IF target IS NULL OR target !~ '^[a-z][a-z0-9._-]{0,31}$' THEN RAISE EXCEPTION 'invalid' USING ERRCODE='22023'; END IF;
 IF new_services IS NULL OR cardinality(new_services) NOT BETWEEN 1 AND 50 OR NOT(new_services <@ ceiling)
  OR EXISTS(SELECT 1 FROM unnest(new_services) s WHERE s IS NULL OR s !~ '^[A-Za-z0-9_-]{1,128}$') THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(o,target,'identity')::text,0));
 PERFORM public.wk_admin_confirm(p,ceiling);
 IF EXISTS(SELECT 1 FROM public.principal_reservations WHERE org=o AND user_id=target)
  OR EXISTS(SELECT 1 FROM public.identities WHERE org=o AND user_id=target) OR EXISTS(SELECT 1 FROM public.memberships WHERE org=o AND user_id=target) THEN
  RAISE EXCEPTION 'conflict' USING ERRCODE='23505';
 END IF;
 INSERT INTO public.principal_reservations(org,user_id) VALUES(o,target);
 INSERT INTO public.identities(issuer,subject,org,user_id) VALUES('wk-admin-created',gen_random_uuid()::text,o,target);
 INSERT INTO public.memberships(org,product,user_id,services,can_review) VALUES(o,p,target,new_services,review);
 INSERT INTO public.audit_events(org,product,user_id,action,entity_id,target_user) VALUES(o,p,who,'admin_user_create','00000000-0000-0000-0000-000000000000',target);
END; $$;

-- Removal revokes credentials and membership, while retaining the identity and name reservation.
-- Re-adding the same known principal is an explicit membership change; creation never adopts it.
CREATE OR REPLACE FUNCTION wk_admin_delete_user(p TEXT,target TEXT) RETURNS VOID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ceiling TEXT[]; o TEXT:=current_setting('wk.org',true);
BEGIN
 ceiling:=public.wk_admin_ceiling(p); PERFORM public.wk_admin_confirm(p,ceiling);
 IF target IS NULL OR NOT EXISTS(SELECT 1 FROM public.identities i WHERE i.org=o AND i.user_id=target AND i.issuer='wk-admin-created')
  OR EXISTS(SELECT 1 FROM public.identities i WHERE i.org=o AND i.user_id=target AND i.issuer<>'wk-admin-created') THEN
  RETURN;
 END IF;
 IF EXISTS(SELECT 1 FROM public.memberships WHERE org=o AND user_id=target) THEN RETURN; END IF;
 DELETE FROM public.personal_tokens WHERE org=o AND user_id=target;
 INSERT INTO public.principal_reservations(org,user_id) VALUES(o,target) ON CONFLICT DO NOTHING;
 INSERT INTO public.audit_events(org,product,user_id,action,entity_id,target_user) VALUES(o,p,current_setting('wk.user',true),'admin_user_delete','00000000-0000-0000-0000-000000000000',target);
END; $$;
