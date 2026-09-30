CREATE TABLE IF NOT EXISTS products (
 org TEXT NOT NULL, id TEXT NOT NULL, PRIMARY KEY(org,id)
);
CREATE TABLE IF NOT EXISTS memberships (
 org TEXT NOT NULL, product TEXT NOT NULL, user_id TEXT NOT NULL,
 services TEXT[] NOT NULL, can_review BOOLEAN NOT NULL DEFAULT FALSE,
 policy_revision BIGINT NOT NULL DEFAULT 1 CHECK(policy_revision>0),
 PRIMARY KEY(org,product,user_id),
 FOREIGN KEY(org,product) REFERENCES products(org,id)
);
CREATE TABLE IF NOT EXISTS cards (
 id UUID PRIMARY KEY, org TEXT NOT NULL, product TEXT NOT NULL, author TEXT NOT NULL,
 idempotency_key TEXT NOT NULL, services TEXT[] NOT NULL CHECK(cardinality(services)>0),
 body JSONB NOT NULL, revision BIGINT NOT NULL DEFAULT 1,
 published BOOLEAN NOT NULL DEFAULT FALSE,
 withdrawn BOOLEAN NOT NULL DEFAULT FALSE,
 UNIQUE(org,product,author,idempotency_key),
 FOREIGN KEY(org,product) REFERENCES products(org,id)
);
CREATE INDEX IF NOT EXISTS cards_scope ON cards(org,product) WHERE published AND NOT withdrawn;
CREATE INDEX IF NOT EXISTS cards_search ON cards USING GIN (to_tsvector('simple', (body->>'title') || ' ' || (body->>'intent') || ' ' || (body->>'rationale')));
