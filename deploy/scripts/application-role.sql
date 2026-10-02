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
