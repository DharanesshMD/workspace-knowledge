-- Repository registration is operator-only; identity metadata confers no membership.
CREATE TABLE code_repositories (
 org TEXT NOT NULL, product TEXT NOT NULL, id UUID NOT NULL,
 services TEXT[] NOT NULL CHECK(cardinality(services) BETWEEN 1 AND 50),
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),
 high_risk_paths TEXT[] NOT NULL DEFAULT '{}', contact_hosts TEXT[] NOT NULL DEFAULT '{}',
 revoked BOOLEAN NOT NULL DEFAULT false,
 PRIMARY KEY(org,product,id), FOREIGN KEY(org,product) REFERENCES products(org,id)
);
CREATE TABLE code_profiles (
 org TEXT NOT NULL, product TEXT NOT NULL, user_id TEXT NOT NULL,
 services TEXT[] NOT NULL CHECK(cardinality(services) BETWEEN 1 AND 50),
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),
 display_name TEXT CHECK(length(display_name) BETWEEN 1 AND 128), contact_url TEXT,
 revoked BOOLEAN NOT NULL DEFAULT false,
 PRIMARY KEY(org,product,user_id), FOREIGN KEY(org,product) REFERENCES products(org,id)
);
CREATE TABLE code_mappings (
 org TEXT NOT NULL, product TEXT NOT NULL, id UUID NOT NULL,
 repository_id UUID NOT NULL, user_id TEXT NOT NULL,
 identity_reference TEXT NOT NULL, proof_reference TEXT NOT NULL,
 services TEXT[] NOT NULL CHECK(cardinality(services) BETWEEN 1 AND 50),
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0), reviewer TEXT NOT NULL,
 assurance TEXT NOT NULL DEFAULT 'team_attested' CHECK(assurance='team_attested'),
 revoked BOOLEAN NOT NULL DEFAULT false,
 PRIMARY KEY(org,product,id), UNIQUE(org,product,repository_id,identity_reference),
 FOREIGN KEY(org,product,repository_id) REFERENCES code_repositories(org,product,id)
);
ALTER TABLE code_repositories ENABLE ROW LEVEL SECURITY;
ALTER TABLE code_profiles ENABLE ROW LEVEL SECURITY;
ALTER TABLE code_mappings ENABLE ROW LEVEL SECURITY;
CREATE POLICY code_repository_read ON code_repositories FOR SELECT USING (
 org=current_setting('wk.org',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_repositories.org AND m.product=code_repositories.product AND m.user_id=current_setting('wk.user',true) AND m.services @> code_repositories.services)
);
CREATE POLICY code_mapping_read ON code_mappings FOR SELECT USING (
 org=current_setting('wk.org',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_mappings.org AND m.product=code_mappings.product AND m.user_id=current_setting('wk.user',true) AND m.services @> code_mappings.services)
 AND EXISTS(SELECT 1 FROM code_repositories r WHERE r.org=code_mappings.org AND r.product=code_mappings.product AND r.id=code_mappings.repository_id AND NOT r.revoked)
);
-- Profile details are not a team directory. Shared attribution uses the scoped function
-- introduced with anchored evidence; direct application SELECT is owner-only.
CREATE POLICY code_profile_owner ON code_profiles FOR ALL USING (
 org=current_setting('wk.org',true) AND user_id=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_profiles.org AND m.product=code_profiles.product AND m.user_id=code_profiles.user_id AND m.services @> code_profiles.services)
) WITH CHECK (
 org=current_setting('wk.org',true) AND user_id=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=code_profiles.org AND m.product=code_profiles.product AND m.user_id=code_profiles.user_id AND m.services @> code_profiles.services)
);
