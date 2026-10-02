-- When each card's work happened, and live "who is working on what" activity.
ALTER TABLE cards ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT now();
CREATE INDEX IF NOT EXISTS cards_evidence ON cards USING GIN ((body->'evidence') jsonb_path_ops);

-- In-progress work signals: intent summary and workspace-relative files only, never prompts.
CREATE TABLE IF NOT EXISTS activity (
 org TEXT NOT NULL, product TEXT NOT NULL, user_id TEXT NOT NULL, session_id TEXT NOT NULL,
 services TEXT[] NOT NULL CHECK(cardinality(services)>0),
 files TEXT[] NOT NULL CHECK(cardinality(files)<=200),
 intent TEXT NOT NULL CHECK(length(intent) BETWEEN 1 AND 280),
 status TEXT NOT NULL CHECK(status IN ('active','ended')),
 started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 last_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL,
 PRIMARY KEY(org,product,user_id,session_id),
 FOREIGN KEY(org,product) REFERENCES products(org,id)
);
CREATE INDEX IF NOT EXISTS activity_files ON activity USING GIN (files);
ALTER TABLE activity ENABLE ROW LEVEL SECURITY;
-- Members see activity only when they hold every service it touches.
CREATE POLICY activity_read ON activity FOR SELECT USING
 (org=current_setting('wk.org',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=activity.org AND m.product=activity.product AND m.user_id=current_setting('wk.user',true) AND m.services @> activity.services));
-- Each developer writes only their own activity, within their membership.
CREATE POLICY activity_write ON activity FOR ALL USING
 (org=current_setting('wk.org',true) AND user_id=current_setting('wk.user',true))
 WITH CHECK
 (org=current_setting('wk.org',true) AND user_id=current_setting('wk.user',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=activity.org AND m.product=activity.product AND m.user_id=activity.user_id AND m.services @> activity.services));
