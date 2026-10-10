-- Global restore cutoff; no bearer token or identity content lives here.
CREATE TABLE collector_security_epoch (
 singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
 not_before BIGINT NOT NULL DEFAULT 0 CHECK(not_before>=0)
);
INSERT INTO collector_security_epoch(singleton,not_before) VALUES(TRUE,0);
