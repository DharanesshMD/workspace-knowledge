-- Optional owner migration. Core PostgreSQL deployments do not run this file.
CREATE EXTENSION IF NOT EXISTS vector;
CREATE TABLE IF NOT EXISTS card_embeddings (
 org TEXT NOT NULL, product TEXT NOT NULL, card_id UUID NOT NULL,
 revision BIGINT NOT NULL CHECK(revision>0),
 model TEXT NOT NULL, digest TEXT NOT NULL CHECK(digest ~ '^[a-f0-9]{64}$'),
 dimensions INTEGER NOT NULL CHECK(dimensions BETWEEN 1 AND 2048),
 format TEXT NOT NULL CHECK(format='nomic-search-v1'),
 embedding vector,
 status TEXT NOT NULL CHECK(status IN ('ready','held')),
 retry_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 PRIMARY KEY(org,product,card_id,model,digest,dimensions,format),
 FOREIGN KEY(org,product,card_id) REFERENCES cards(org,product,id) ON DELETE CASCADE,
 CHECK((status='ready' AND embedding IS NOT NULL AND vector_dims(embedding)=dimensions)
    OR (status='held' AND embedding IS NULL))
);
ALTER TABLE card_embeddings ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS embedding_read ON card_embeddings;
CREATE POLICY embedding_read ON card_embeddings FOR SELECT USING (
 org=current_setting('wk.org',true) AND EXISTS (
  SELECT 1 FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product
  WHERE c.org=card_embeddings.org AND c.product=card_embeddings.product
   AND c.id=card_embeddings.card_id AND c.revision=card_embeddings.revision
   AND m.user_id=current_setting('wk.user',true) AND m.services @> c.services
   AND c.published AND NOT c.withdrawn
   AND c.body->>'lifecycle' NOT IN ('superseded','deprecated')
 )
);
-- Runtime cannot mutate derived data. This narrowly scoped owner trigger keeps
-- ordinary corrections/withdrawals working without granting vector DELETE.
CREATE OR REPLACE FUNCTION wk_clear_card_embeddings() RETURNS trigger
 LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF NEW.revision IS DISTINCT FROM OLD.revision
  OR NEW.published IS DISTINCT FROM OLD.published
  OR NEW.withdrawn IS DISTINCT FROM OLD.withdrawn
  OR NEW.body IS DISTINCT FROM OLD.body THEN
  DELETE FROM public.card_embeddings WHERE org=OLD.org AND product=OLD.product AND card_id=OLD.id;
 END IF;
 RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION wk_clear_card_embeddings() FROM PUBLIC;
DROP TRIGGER IF EXISTS clear_card_embeddings ON cards;
CREATE TRIGGER clear_card_embeddings AFTER UPDATE ON cards
 FOR EACH ROW EXECUTE FUNCTION wk_clear_card_embeddings();
