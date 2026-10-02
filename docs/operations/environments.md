# Environment broker

`wk-env` is a separate binary/crate with no knowledge/capture dependencies. No MCP secret retrieval is provided. Configure a JSON profile containing `service`, `profile` (development/test/staging), `mount`, `path` (ending in service/profile), `address` (HTTPS origin), absolute canonical `workspace`, and `keys` (array). Production profiles are denied; production policy enablement is intentionally not implemented.

Example metadata (no secrets):
```json
{"service":"backend","profile":"development","mount":"secret","path":"team/backend/development","address":"https://bao.internal:8200","workspace":"/absolute/product","keys":["DATABASE_URL"]}
```

Authenticate separately to OpenBao using its supported OIDC/auth workflow. Supply an independently scoped, short-lived `WK_BAO_TOKEN` through your trusted credential launcher, never an argument or knowledge-service token. OpenBao policy must grant exactly the selected KV v2 data path, not wildcard access. Server policy is authoritative; editing local JSON grants no access. Tokens and values are never logged. TLS certificate validation is mandatory and redirects are disabled. For a private CA, configure `WK_BAO_CA_FILE` with its PEM certificate; hostname and certificate verification remain enabled. Never disable verification.

```
wk-env --config profile.json schema
wk-env --config profile.json import --file /absolute/product/.env --approve-file-import
wk-env --config profile.json run -- npm run dev
wk-env --config profile.json export --destination /absolute/product/.env.local
```

Schema is metadata-only and does not claim vault readiness. Review its key list before approving import. Import accepts literal `KEY=value` lines and comments, without shell interpolation, quote decoding, or multiline values. CAS=0 prevents overwriting an existing KV secret. Rotate through approved vault administration; every run fetches current values and a revoked token fails on the next request. KV values are static secrets, not revocable dynamic leases; this release does not implement dynamic secret issuance or kill running children after revocation.

Export is explicit, create-only, inside the canonical workspace, Git-ignored and untracked. POSIX mode is 0600. Windows exports fail closed because verified ACL support is not implemented. Parent-directory races by an attacker with the same OS identity remain outside the threat boundary; use directories owned and writable only by the developer. Newlines are refused. Child commands use an argument array, and the vault token is removed from inherited environment. Exact known secret echoes in stdout/stderr are suppressed, with bounded buffered output (1 MiB per stream); interactive terminal applications may not work. Transformations, child-created files, process memory, inherited unrelated credentials, and network exfiltration are outside this guarantee. Run only trusted commands and keep broker sessions outside capture.

Official API: [KV v2](https://openbao.org/api-docs/secret/kv/kv-v2/) and [token auth](https://openbao.org/api-docs/auth/token/). The disposable development compose uses generated TLS certificates. Copy `/tmp/wk-tls/vault-ca.pem` from its container and set `WK_BAO_CA_FILE`; the test root token is never suitable for real deployments.

`python3 scripts/environment-integration.py` exercises a disposable container named `wk-final-bao` (see CI for startup). It uses synthetic values only and verifies TLS, scoped-token import, CAS overwrite refusal, child delivery without the broker token, exact-value output suppression, rotation, and denial after revocation. This passed locally on macOS arm64 with OpenBao 2.4.3. Remote CI and production-vault authentication remain unverified.
