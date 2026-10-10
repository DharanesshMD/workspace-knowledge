-- Run as migration owner. Create a LOGIN role wk_app with your secret-management
-- system first. Never use wk_app as a migration owner or grant it BYPASSRLS.
GRANT USAGE ON SCHEMA public TO wk_app;
GRANT SELECT ON identities,products,memberships TO wk_app;
-- SELECT FOR SHARE on memberships additionally requires UPDATE privilege on a
-- column. RLS has no UPDATE policy, so this does not authorize membership edits.
GRANT UPDATE(policy_revision) ON memberships TO wk_app;
GRANT SELECT,INSERT,UPDATE ON cards,workspace_bindings,card_revisions,feedback,tombstones TO wk_app;
GRANT SELECT,INSERT,UPDATE,DELETE ON graph_nodes,graph_edges TO wk_app;
GRANT SELECT,UPDATE(stale) ON trusted_evidence TO wk_app;
GRANT INSERT ON audit_events TO wk_app;
GRANT USAGE ON SEQUENCE audit_events_id_seq TO wk_app;
GRANT SELECT,INSERT,UPDATE,DELETE ON activity TO wk_app;
-- Authentication resolves identity before an Actor exists. Only opaque session
-- hashes are stored; these rows are accessible solely through the auth service.
GRANT SELECT,INSERT,UPDATE,DELETE ON browser_sessions TO wk_app;

GRANT SELECT,INSERT,UPDATE ON personal_summaries TO wk_app;
GRANT EXECUTE ON FUNCTION wk_personal_has_capacity(TEXT,TEXT,TEXT) TO wk_app;

GRANT SELECT,INSERT,UPDATE ON registered_devices,device_reports TO wk_app;
GRANT SELECT,INSERT,DELETE ON device_nonces TO wk_app;
GRANT EXECUTE ON FUNCTION wk_device_has_capacity(TEXT,TEXT) TO wk_app;

GRANT SELECT,INSERT,DELETE ON collector_credentials TO wk_app;

GRANT SELECT ON product_settings TO wk_app;
GRANT EXECUTE ON FUNCTION wk_admin_members(TEXT),wk_admin_member(TEXT,TEXT,TEXT[],BOOLEAN,BOOLEAN),wk_admin_device(TEXT,UUID),wk_admin_policy(TEXT,JSONB) TO wk_app;
