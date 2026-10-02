-- Defense in depth for non-owner application roles. Migration/admin owners bypass
-- RLS by PostgreSQL design; they must never be used as the HTTP application role.
ALTER TABLE memberships ENABLE ROW LEVEL SECURITY;
CREATE POLICY membership_identity ON memberships FOR SELECT USING
 (org=current_setting('wk.org',true) AND user_id=current_setting('wk.user',true));
ALTER TABLE products ENABLE ROW LEVEL SECURITY;
CREATE POLICY product_membership ON products FOR SELECT USING
 (org=current_setting('wk.org',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=products.org AND m.product=products.id));
ALTER TABLE cards ENABLE ROW LEVEL SECURITY;
CREATE POLICY card_membership ON cards USING
 (org=current_setting('wk.org',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=cards.org AND m.product=cards.product AND m.services @> cards.services AND (cards.published OR cards.author=m.user_id OR m.can_review)))
 WITH CHECK
 (org=current_setting('wk.org',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=cards.org AND m.product=cards.product AND m.services @> cards.services AND (cards.author=m.user_id OR m.can_review)));
ALTER TABLE workspace_bindings ENABLE ROW LEVEL SECURITY;
CREATE POLICY binding_identity ON workspace_bindings USING
 (org=current_setting('wk.org',true) AND user_id=current_setting('wk.user',true))
 WITH CHECK
 (org=current_setting('wk.org',true) AND user_id=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=workspace_bindings.org AND m.product=workspace_bindings.product AND m.services @> workspace_bindings.services AND m.policy_revision=workspace_bindings.policy_revision));
ALTER TABLE card_revisions ENABLE ROW LEVEL SECURITY;
CREATE POLICY revision_membership ON card_revisions USING
 (EXISTS(SELECT 1 FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product WHERE c.id=card_revisions.card_id AND c.org=card_revisions.org AND c.product=card_revisions.product AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements_text(card_revisions.body->'service_ids') s WHERE NOT s=ANY(m.services))));
ALTER TABLE feedback ENABLE ROW LEVEL SECURITY;
CREATE POLICY feedback_membership ON feedback USING
 (EXISTS(SELECT 1 FROM cards c WHERE c.id=feedback.card_id AND c.org=feedback.org AND c.product=feedback.product));
ALTER TABLE graph_nodes ENABLE ROW LEVEL SECURITY;
CREATE POLICY graph_node_membership ON graph_nodes USING
 (EXISTS(SELECT 1 FROM cards c WHERE c.id=graph_nodes.card_id AND c.org=graph_nodes.org AND c.product=graph_nodes.product));
ALTER TABLE graph_edges ENABLE ROW LEVEL SECURITY;
CREATE POLICY graph_edge_membership ON graph_edges USING
 (EXISTS(SELECT 1 FROM graph_nodes n WHERE n.id=graph_edges.source AND n.org=graph_edges.org AND n.product=graph_edges.product) AND EXISTS(SELECT 1 FROM graph_nodes n WHERE n.id=graph_edges.target AND n.org=graph_edges.org AND n.product=graph_edges.product));
ALTER TABLE trusted_evidence ENABLE ROW LEVEL SECURITY;
CREATE POLICY evidence_membership ON trusted_evidence USING
 (EXISTS(SELECT 1 FROM cards c WHERE c.id=trusted_evidence.card_id AND c.org=trusted_evidence.org AND c.product=trusted_evidence.product));
ALTER TABLE tombstones ENABLE ROW LEVEL SECURITY;
CREATE POLICY tombstone_membership ON tombstones USING
 (org=current_setting('wk.org',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=tombstones.org AND m.product=tombstones.product));
ALTER TABLE audit_events ENABLE ROW LEVEL SECURITY;
CREATE POLICY audit_identity ON audit_events USING
 (org=current_setting('wk.org',true) AND user_id=current_setting('wk.user',true));
-- Row locking for authorization needs UPDATE visibility, but no membership mutation.
CREATE POLICY membership_lock ON memberships FOR UPDATE USING
 (org=current_setting('wk.org',true) AND user_id=current_setting('wk.user',true)) WITH CHECK (false);
