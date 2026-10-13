-- Independently retained body-free deletion journal; may reference post-backup products.
CREATE TABLE personal_deletions (
 org TEXT NOT NULL, product TEXT NOT NULL, owner TEXT NOT NULL, note_id UUID NOT NULL,
 revision BIGINT NOT NULL CHECK(revision>0), key_hash TEXT NOT NULL CHECK(key_hash ~ '^[0-9a-f]{64}$'), deleted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 PRIMARY KEY(org,product,owner,key_hash), UNIQUE(org,product,owner,note_id)
);
ALTER TABLE personal_deletions ENABLE ROW LEVEL SECURITY;
CREATE FUNCTION wk_journal_personal_deletion() RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF NEW.deleted THEN
  INSERT INTO public.personal_deletions(org,product,owner,note_id,revision,key_hash) VALUES(NEW.org,NEW.product,NEW.owner,NEW.id,NEW.revision,encode(sha256(convert_to(NEW.idempotency_key,'UTF8')),'hex'))
   ON CONFLICT(org,product,owner,key_hash) DO UPDATE SET revision=GREATEST(personal_deletions.revision,EXCLUDED.revision),deleted_at=GREATEST(personal_deletions.deleted_at,EXCLUDED.deleted_at);
 END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER personal_deletion_journal AFTER INSERT OR UPDATE ON personal_summaries FOR EACH ROW EXECUTE FUNCTION wk_journal_personal_deletion();
INSERT INTO personal_deletions(org,product,owner,note_id,revision,key_hash) SELECT org,product,owner,id,revision,encode(sha256(convert_to(idempotency_key,'UTF8')),'hex') FROM personal_summaries WHERE deleted;
CREATE FUNCTION wk_personal_deleted_key(o TEXT,p TEXT,who TEXT,key_value TEXT) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF current_setting('wk.org',true) IS DISTINCT FROM o OR current_setting('wk.user',true) IS DISTINCT FROM who THEN RAISE EXCEPTION 'denied' USING ERRCODE='42501'; END IF;
 RETURN EXISTS(SELECT 1 FROM public.personal_deletions WHERE org=o AND product=p AND owner=who AND key_hash=encode(sha256(convert_to(key_value,'UTF8')),'hex'));
END; $$;
REVOKE ALL ON FUNCTION wk_journal_personal_deletion(),wk_personal_deleted_key(TEXT,TEXT,TEXT,TEXT) FROM PUBLIC;
CREATE INDEX report_expiration ON device_reports(expires_at);
CREATE INDEX activity_expiration ON activity(expires_at);
CREATE INDEX browser_idle_expiration ON browser_sessions(last_seen);
