-- Files an assistant reported when its developer shared a session summary (share_session). They are what
-- lets a teammate ask about a file or a module. They are agent-reported, not observed, so they live in
-- their own table instead of the card's evidence, and the card contract is unchanged.
-- Row-level security mirrors the cards table's membership rule: a row is only visible to a member of the
-- product, and the card's own author is the only one who can add rows (checked by the INSERT itself).
CREATE TABLE card_files (
 org TEXT NOT NULL, product TEXT NOT NULL, card_id UUID NOT NULL,
 path TEXT NOT NULL CHECK(char_length(path) BETWEEN 1 AND 512),
 PRIMARY KEY(org,product,card_id,path),
 FOREIGN KEY(org,product,card_id) REFERENCES cards(org,product,id) ON DELETE CASCADE
);
ALTER TABLE card_files ENABLE ROW LEVEL SECURITY;
CREATE POLICY card_files_read ON card_files FOR SELECT USING
 (org=current_setting('wk.org',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=card_files.org AND m.product=card_files.product AND m.user_id=current_setting('wk.user',true)));
CREATE POLICY card_files_write ON card_files FOR INSERT WITH CHECK
 (org=current_setting('wk.org',true) AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=card_files.org AND m.product=card_files.product AND m.user_id=current_setting('wk.user',true)));
