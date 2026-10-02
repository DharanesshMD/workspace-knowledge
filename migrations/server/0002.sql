CREATE TABLE IF NOT EXISTS identities (
 issuer TEXT NOT NULL, subject TEXT NOT NULL, org TEXT NOT NULL, user_id TEXT NOT NULL,
 PRIMARY KEY(issuer,subject)
);
CREATE TABLE IF NOT EXISTS workspace_bindings (
 id UUID PRIMARY KEY, org TEXT NOT NULL, product TEXT NOT NULL, user_id TEXT NOT NULL,
 device_id TEXT NOT NULL, approval_hash TEXT NOT NULL, services TEXT[] NOT NULL,
 policy_revision BIGINT NOT NULL, revoked BOOLEAN NOT NULL DEFAULT FALSE,
 UNIQUE(org,product,user_id,device_id,approval_hash),
 FOREIGN KEY(org,product,user_id) REFERENCES memberships(org,product,user_id) ON DELETE CASCADE
);
ALTER TABLE cards ADD CONSTRAINT cards_tenant_identity UNIQUE(org,product,id);
CREATE TABLE card_revisions (
 org TEXT NOT NULL, product TEXT NOT NULL, card_id UUID NOT NULL, revision BIGINT NOT NULL,
 body JSONB NOT NULL, changed_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 PRIMARY KEY(org,product,card_id,revision),
 FOREIGN KEY(org,product,card_id) REFERENCES cards(org,product,id)
);
CREATE TABLE tombstones (
 org TEXT NOT NULL, product TEXT NOT NULL, card_id UUID NOT NULL, revision BIGINT NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY(org,product,card_id)
);
CREATE TABLE feedback (
 id UUID PRIMARY KEY, org TEXT NOT NULL, product TEXT NOT NULL, card_id UUID NOT NULL,
 revision BIGINT NOT NULL, user_id TEXT NOT NULL, issue TEXT NOT NULL, suggestion TEXT NOT NULL,
 FOREIGN KEY(org,product,card_id) REFERENCES cards(org,product,id)
);
CREATE TABLE graph_nodes (
 id UUID PRIMARY KEY, org TEXT NOT NULL, product TEXT NOT NULL, card_id UUID NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('product','service','feature','decision','claim','evidence')),
 label TEXT NOT NULL, provenance JSONB NOT NULL,
 UNIQUE(org,product,id), FOREIGN KEY(org,product,card_id) REFERENCES cards(org,product,id)
);
CREATE TABLE graph_edges (
 org TEXT NOT NULL, product TEXT NOT NULL, source UUID NOT NULL, target UUID NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('belongs_to','affects','implements','depends_on','supports','contradicts','supersedes')),
 PRIMARY KEY(org,product,source,target,kind),
 FOREIGN KEY(org,product,source) REFERENCES graph_nodes(org,product,id) ON DELETE CASCADE,
 FOREIGN KEY(org,product,target) REFERENCES graph_nodes(org,product,id) ON DELETE CASCADE
);
CREATE TABLE trusted_evidence (
 id UUID PRIMARY KEY, org TEXT NOT NULL, product TEXT NOT NULL, card_id UUID NOT NULL,
 card_revision BIGINT NOT NULL, provider TEXT NOT NULL, reference TEXT NOT NULL,
 code_revision TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('code_observed','tests_verified','owner_confirmed')),
 metadata JSONB NOT NULL, stale BOOLEAN NOT NULL DEFAULT FALSE,
 FOREIGN KEY(org,product,card_id) REFERENCES cards(org,product,id)
);
CREATE TABLE audit_events (
 id BIGSERIAL PRIMARY KEY, org TEXT NOT NULL, product TEXT NOT NULL, user_id TEXT NOT NULL,
 action TEXT NOT NULL, entity_id UUID NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
