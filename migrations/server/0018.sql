-- Lexical search used to tokenize every candidate card on every request. Under
-- the restricted application role, row-level security keeps PostgreSQL from
-- using a GIN index for the non-leakproof @@ operator, so cards_search was never
-- used at runtime and the cost grew with the cards in scope (about 1.3 ms per
-- 10 KiB card). Store the vector with the card so a request only matches it.
-- Generated columns follow every later edit to the searchable fields.
DROP INDEX IF EXISTS cards_search;
ALTER TABLE cards ADD COLUMN search_vector tsvector GENERATED ALWAYS AS (
 to_tsvector('simple',(body->>'title') || ' ' || (body->>'intent') || ' ' || (body->>'rationale') || ' ' || COALESCE(body->>'decisions','') || ' ' || COALESCE(body->>'constraints','') || ' ' || COALESCE(body->>'open_questions',''))
) STORED;
-- Publication looks up a card's graph nodes by (org, product, card_id), but the
-- foreign key to cards created no index on the referencing side, so every
-- publication scanned all graph_nodes (87 ms per publication at 24,000 nodes,
-- growing with the table until the single-writer product gate queued requests).
CREATE INDEX graph_nodes_card ON graph_nodes(org,product,card_id);
