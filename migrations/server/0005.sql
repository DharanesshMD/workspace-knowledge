-- Search all reviewable knowledge fields, using the same expression as retrieval.
DROP INDEX IF EXISTS cards_search;
CREATE INDEX cards_search ON cards USING GIN (to_tsvector('simple', (body->>'title') || ' ' || (body->>'intent') || ' ' || (body->>'rationale') || ' ' || COALESCE(body->>'decisions','') || ' ' || COALESCE(body->>'constraints','') || ' ' || COALESCE(body->>'open_questions','')));
