-- Explicit optional grant, run only after semantic-migrate with owner credentials.
-- No vector writes or model configuration permission is granted to the runtime.
GRANT SELECT ON card_embeddings TO wk_app;
