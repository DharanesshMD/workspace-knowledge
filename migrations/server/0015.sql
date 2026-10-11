CREATE TABLE code_contributions (
 org TEXT NOT NULL, product TEXT NOT NULL, id UUID NOT NULL,
 repository_id UUID NOT NULL, repository_revision BIGINT NOT NULL,
 mapping_id UUID NOT NULL, mapping_revision BIGINT NOT NULL,
 owner TEXT NOT NULL, submitted_by TEXT NOT NULL,
 code_revision TEXT NOT NULL CHECK(code_revision ~ '^([a-f0-9]{40}|[a-f0-9]{64})$'),
 targets JSONB NOT NULL CHECK(jsonb_typeof(targets)='array' AND jsonb_array_length(targets) BETWEEN 1 AND 20),
 services TEXT[] NOT NULL CHECK(cardinality(services) BETWEEN 1 AND 50),
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),
 confirmed_by TEXT, confirmed_at_ms BIGINT, reviewed_by TEXT, reviewed_at_ms BIGINT,
 revoked BOOLEAN NOT NULL DEFAULT false,
 PRIMARY KEY(org,product,id),
 FOREIGN KEY(org,product,repository_id) REFERENCES code_repositories(org,product,id),
 FOREIGN KEY(org,product,mapping_id) REFERENCES code_mappings(org,product,id),
 CHECK(confirmed_by IS NULL OR confirmed_by=owner)
);
CREATE TABLE code_rationale_anchors (
 org TEXT NOT NULL, product TEXT NOT NULL, id UUID NOT NULL,
 repository_id UUID NOT NULL, repository_revision BIGINT NOT NULL,
 card_id UUID NOT NULL REFERENCES cards(id), card_revision BIGINT NOT NULL,
 contribution_id UUID, contribution_revision BIGINT,
 owner TEXT NOT NULL, submitted_by TEXT NOT NULL,
 code_revision TEXT NOT NULL CHECK(code_revision ~ '^([a-f0-9]{40}|[a-f0-9]{64})$'),
 targets JSONB NOT NULL CHECK(jsonb_typeof(targets)='array' AND jsonb_array_length(targets) BETWEEN 1 AND 20),
 services TEXT[] NOT NULL CHECK(cardinality(services) BETWEEN 1 AND 50),
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),
 confirmed_by TEXT, confirmed_at_ms BIGINT, reviewed_by TEXT, reviewed_at_ms BIGINT,
 revoked BOOLEAN NOT NULL DEFAULT false,
 PRIMARY KEY(org,product,id),
 FOREIGN KEY(org,product,repository_id) REFERENCES code_repositories(org,product,id),
 FOREIGN KEY(org,product,contribution_id) REFERENCES code_contributions(org,product,id),
 CHECK(confirmed_by IS NULL OR confirmed_by=owner),
 CHECK((contribution_id IS NULL)=(contribution_revision IS NULL))
);
CREATE INDEX code_anchor_lookup ON code_rationale_anchors(org,product,repository_id,code_revision) WHERE NOT revoked;
CREATE INDEX code_anchor_targets ON code_rationale_anchors USING GIN(targets);
CREATE TABLE code_maintainers (
 org TEXT NOT NULL, product TEXT NOT NULL, id UUID NOT NULL,
 repository_id UUID NOT NULL, user_id TEXT NOT NULL,
 targets JSONB NOT NULL CHECK(jsonb_typeof(targets)='array' AND jsonb_array_length(targets) BETWEEN 1 AND 20),
 services TEXT[] NOT NULL CHECK(cardinality(services) BETWEEN 1 AND 50),
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0), reviewer TEXT NOT NULL,
 revoked BOOLEAN NOT NULL DEFAULT false,
 PRIMARY KEY(org,product,id),
 FOREIGN KEY(org,product,repository_id) REFERENCES code_repositories(org,product,id)
);
ALTER TABLE code_contributions ENABLE ROW LEVEL SECURITY;
ALTER TABLE code_rationale_anchors ENABLE ROW LEVEL SECURITY;
ALTER TABLE code_maintainers ENABLE ROW LEVEL SECURITY;
CREATE POLICY code_contribution_scope ON code_contributions FOR ALL USING (
 org=current_setting('wk.org',true)
 AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_contributions.org AND m.product=code_contributions.product AND m.user_id=current_setting('wk.user',true) AND m.services @> code_contributions.services AND (code_contributions.owner=m.user_id OR code_contributions.submitted_by=m.user_id OR m.can_review OR (code_contributions.reviewed_by IS NOT NULL AND code_contributions.confirmed_by IS NOT NULL)))
 AND EXISTS(SELECT 1 FROM code_repositories r WHERE r.org=code_contributions.org AND r.product=code_contributions.product AND r.id=code_contributions.repository_id AND NOT r.revoked)
) WITH CHECK (
 org=current_setting('wk.org',true)
 AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_contributions.org AND m.product=code_contributions.product AND m.user_id=current_setting('wk.user',true) AND m.services @> code_contributions.services AND (code_contributions.owner=m.user_id OR code_contributions.submitted_by=m.user_id OR m.can_review))
 AND EXISTS(SELECT 1 FROM code_repositories r WHERE r.org=code_contributions.org AND r.product=code_contributions.product AND r.id=code_contributions.repository_id AND NOT r.revoked)
);
CREATE POLICY code_anchor_scope ON code_rationale_anchors FOR ALL USING (
 org=current_setting('wk.org',true)
 AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_rationale_anchors.org AND m.product=code_rationale_anchors.product AND m.user_id=current_setting('wk.user',true) AND m.services @> code_rationale_anchors.services AND (code_rationale_anchors.owner=m.user_id OR code_rationale_anchors.submitted_by=m.user_id OR m.can_review OR (code_rationale_anchors.reviewed_by IS NOT NULL AND code_rationale_anchors.confirmed_by IS NOT NULL)))
 AND EXISTS(SELECT 1 FROM code_repositories r WHERE r.org=code_rationale_anchors.org AND r.product=code_rationale_anchors.product AND r.id=code_rationale_anchors.repository_id AND NOT r.revoked)
 AND EXISTS(SELECT 1 FROM cards c WHERE c.org=code_rationale_anchors.org AND c.product=code_rationale_anchors.product AND c.id=code_rationale_anchors.card_id AND NOT c.withdrawn AND (c.published OR c.author=current_setting('wk.user',true) OR EXISTS(SELECT 1 FROM memberships m WHERE m.org=c.org AND m.product=c.product AND m.user_id=current_setting('wk.user',true) AND m.can_review AND m.services @> c.services)))
) WITH CHECK (
 org=current_setting('wk.org',true)
 AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_rationale_anchors.org AND m.product=code_rationale_anchors.product AND m.user_id=current_setting('wk.user',true) AND m.services @> code_rationale_anchors.services AND (code_rationale_anchors.owner=m.user_id OR code_rationale_anchors.submitted_by=m.user_id OR m.can_review))
);
CREATE POLICY code_maintainer_read ON code_maintainers FOR SELECT USING (
 org=current_setting('wk.org',true)
 AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_maintainers.org AND m.product=code_maintainers.product AND m.user_id=current_setting('wk.user',true) AND m.services @> code_maintainers.services)
 AND EXISTS(SELECT 1 FROM code_repositories r WHERE r.org=code_maintainers.org AND r.product=code_maintainers.product AND r.id=code_maintainers.repository_id AND NOT r.revoked)
);
-- Only profiles attached to approved contribution/rationale/maintainer evidence
-- can be selected by another user. There is no standalone team profile directory.
CREATE POLICY code_profile_evidenced_read ON code_profiles FOR SELECT USING (
 org=current_setting('wk.org',true) AND NOT revoked
 AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_profiles.org AND m.product=code_profiles.product AND m.user_id=current_setting('wk.user',true) AND m.services @> code_profiles.services)
 AND (EXISTS(SELECT 1 FROM code_contributions c JOIN code_mappings cm ON cm.org=c.org AND cm.product=c.product AND cm.id=c.mapping_id AND cm.revision=c.mapping_revision AND NOT cm.revoked AND cm.user_id=c.owner JOIN code_repositories cr ON cr.org=c.org AND cr.product=c.product AND cr.id=c.repository_id AND cr.revision=c.repository_revision AND NOT cr.revoked WHERE c.org=code_profiles.org AND c.product=code_profiles.product AND c.owner=code_profiles.user_id AND c.confirmed_by=c.owner AND c.reviewed_by IS NOT NULL AND NOT c.revoked)
 OR EXISTS(SELECT 1 FROM code_rationale_anchors a JOIN cards c ON c.org=a.org AND c.product=a.product AND c.id=a.card_id AND c.revision=a.card_revision AND c.published AND NOT c.withdrawn JOIN code_repositories cr ON cr.org=a.org AND cr.product=a.product AND cr.id=a.repository_id AND cr.revision=a.repository_revision AND NOT cr.revoked WHERE a.org=code_profiles.org AND a.product=code_profiles.product AND a.owner=code_profiles.user_id AND a.confirmed_by=a.owner AND a.reviewed_by IS NOT NULL AND NOT a.revoked)
 OR EXISTS(SELECT 1 FROM code_maintainers m WHERE m.org=code_profiles.org AND m.product=code_profiles.product AND m.user_id=code_profiles.user_id AND NOT m.revoked))
);

-- Validation needs row locks; operator metadata remains immutable to wk_app.
CREATE POLICY code_repositories_lock ON code_repositories FOR UPDATE USING (
 org=current_setting('wk.org',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_repositories.org AND m.product=code_repositories.product AND m.user_id=current_setting('wk.user',true) AND m.services @> code_repositories.services)
) WITH CHECK (false);
CREATE POLICY code_mappings_lock ON code_mappings FOR UPDATE USING (
 org=current_setting('wk.org',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_mappings.org AND m.product=code_mappings.product AND m.user_id=current_setting('wk.user',true) AND m.services @> code_mappings.services)
 AND EXISTS(SELECT 1 FROM code_repositories r WHERE r.org=code_mappings.org AND r.product=code_mappings.product AND r.id=code_mappings.repository_id AND NOT r.revoked)
) WITH CHECK (false);
CREATE POLICY code_maintainers_lock ON code_maintainers FOR UPDATE USING (
 org=current_setting('wk.org',true)
 AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_maintainers.org AND m.product=code_maintainers.product AND m.user_id=current_setting('wk.user',true) AND m.services @> code_maintainers.services)
 AND EXISTS(SELECT 1 FROM code_repositories r WHERE r.org=code_maintainers.org AND r.product=code_maintainers.product AND r.id=code_maintainers.repository_id AND NOT r.revoked)
) WITH CHECK (false);
