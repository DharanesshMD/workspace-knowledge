# Hosted portal operations

Implementation status: scoped administration, bounded physical cleanup and deletion-safe encrypted restore are tested locally. Deployment limits and a five-developer synthetic Linux-container pilot passed local acceptance; see `docs/reviews/2026-10-10-hosted-operations-acceptance.md` for measured evidence and final review status. Production identity/TLS and deployment-specific capacity remain separate gates.

## Administrative boundaries

A reviewer can review authorized shared knowledge. Administration requires a separate explicit organization/product assignment, with a service ceiling set by a protected deployment operator. Administrators receive no additional private-summary or unpublished-card access. A partial-service administrator can list/manage only members and devices whose whole scope is inside its ceiling. Product-wide policy changes require authority over every current service in that product.

The existing `provision` CLI maps upstream issuer/subject to an internal organization/user and grants initial membership through the protected operator database connection. It is not exposed to HTTP or MCP. After membership exists, use `wk-server admin-assign` with bounded JSON stdin containing `org`, `user`, `product`, and `services`. Use a dedicated protected operator connection; the application role cannot assign administrative authority or map identities. Scope narrowing removes broader administrative assignments and revokes affected installation keys. Regranting membership cannot resurrect those keys.

`wk-server admin` uses the application database connection plus ordinary configured OIDC issuer/audience/JWKS. It consumes a private JSON request on stdin with `token`, `product`, and a tagged `command`. The actor is resolved from the verified upstream token; an `actor` argument is rejected. Collector-audience credentials cannot administer. Command kinds are `members`, `member-put` with a typed `member`, `member-remove` with `user`, `policy-put` with typed `policy`, and `device-revoke` with installation `id`. There is no shell/SQL/URL command or identity/role-grant command. Keep credential-bearing request files private (0600) or pipe them from the secret manager; never put tokens in process arguments or public command examples.

Initial metadata lists are bounded to100 scoped memberships. There is no hosted administrative UI in this increment. Successful commands return metadata/boolean acknowledgment; errors are opaque and never echo tokens or request bodies. Membership and policy changes are audited without summary text; privileged bootstrap events record the database role, while authenticated administrative events record the resolved user. Use individual operator database identities for operator accountability.

## Effective restrictions

Product policy contains `private_days`1–30, `report_days`1–7, `allow_loopback`, and up to16 exact HTTPS inference origins. Defaults are30days/7days/local-loopback enabled/no HTTPS origins. Administrators may only shorten retention or remove destinations. Existing expiries are reduced and creation-relative expiry is enforced for new private summaries; edits/retries never extend expiry. Device reports retain only the latest bounded report and use report-relative retention.

Only protected operator `admin-policy-bootstrap` can widen a ceiling, using stdin `org`, `user`, `product`, and typed `policy`. It is audited and cannot change an existing local approval receipt or redirect inference. The collector's existing summarizer still accepts only literal-loopback HTTP inference; an HTTPS origin listed in server policy does not add HTTPS inference support. New or broader destinations require a separate local implementation/approval. Hosted signed policy must include explicit restrictions; unsigned loopback development fixtures retain local defaults.

## Local acceptance

The local upstream public-client PKCE provider issues an ordinary signed access token for the admin test. `scripts/portal-admin-smoke.py --public-client` exercises the actual CLI, database and browser: explicit role assignment, reviewer denial, product/service denial, private-summary denial, narrowed policy/new-note expiry and membership removal. `--dev-admin` is explicitly isolated to databases named `wk_test`/`wk_dev` and literal-loopback fixture issuer/JWKS. It is not a production sign-in fallback. The selected production provider is still pending.

The real signed native device/browser acceptance remains separate and does not certify broader conversation capture, system-browser dispatch or Windows/Linux Keychain equivalents.

## Physical retention worker

Run `wk-server retention-worker` as a separate operator process with a protected connection that owns the migrated deletion journal (or has explicitly trusted owner-role membership). The application connection is refused. `retention-once` runs one bounded sweep and emits counts only. Every60seconds the worker clears up to1000 expired private bodies and at most1000 expired rows per operational table in one transaction. It removes reports, activity, absolute/idle-expired browser sessions, signed-request nonces, expired collector credential bindings and audit events older90days. A single advisory worker lock avoids overlapping sweeps; locked rows are skipped. Storage failures retry on the next scheduled interval; permanent permission failure exits. Shutdown cancels waiting/processing work so an uncommitted batch rolls back.

Logical expiry remains immediate even if the worker is delayed. Physical erasure here means live PostgreSQL text/rows are cleared; WAL, freed database pages and encrypted backups have independent retention and are not instantly erased. Private deletion writes a body-free journal in the same transaction, including only scope, note ID, revision, hashed idempotency key and deletion time. Retries of a deleted key are denied even when the original note was absent from an old snapshot. No application role can read/write that journal or invoke cleanup/replay. `replay-personal-deletions` is an offline operator command.

Keep personal idempotency markers, independent deletion journals and revoked installation-key tombstones. Their metadata currently grows indefinitely; archive only with a defined backup/replay horizon. Unexpired collector bindings must survive through token expiry plus120seconds. The newest independently encrypted journal must cover every backup still retained.

## Deployment bounds

The API validates these per-process settings before serving. Defaults: `WK_POOL_MAX=10` (1–32), `WK_DB_ACQUIRE_MS=5000` (100–10000), `WK_DB_STATEMENT_MS=5000` (100–15000), `WK_DB_LOCK_MS=3000` (100–5000, no greater than statement), `WK_MAX_INFLIGHT=64` (1–64), `WK_HTTP_TIMEOUT_SECONDS=30` (1–30). Acquisition plus statement must fit within the HTTP deadline. Production uses the configured pool; protected operator commands use the same default SQL deadlines. Size total database connections across every API instance, admin process and worker, not just one pool.

Excess API requests receive opaque503 immediately instead of an unbounded queue. The deadline covers response body completion; JSON API/stateless MCP output is buffered up to8MiB. A timed-out write may have committed before cancellation, so retry the same idempotency key or reload a revision instead of inventing another write. New streaming protocols require their own bounds.

`/health/live` reports only process liveness. `/health/ready` requires exactly schema migrations1–13 through the restricted runtime connection, with one concurrent probe and a1-second deadline. It returns only readiness or opaque503. A running retention worker does not make readiness claim backups, capture coverage, issuer health or model availability. `wk-server healthcheck` checks only fixed localhost:8080 readiness, disables proxy/redirect forwarding and needs no database credential of its own. The legacy `/health` remains for compatibility; use readiness to decide whether to route traffic.

Compose starts with app2CPU/1GiB, PostgreSQL2CPU/2GiB,256-process caps,50 database connections,256MiB shared buffers and4MiB work memory. App remains non-root/read-only/cap-drop/no-new-privileges; PostgreSQL is private and app port binds host loopback. Override CPU/RAM deliberately after measurement. These are resource ceilings, not a proven minimum server size. A same-origin HTTPS reverse proxy still needs connection/header/body deadlines and rate limits; application request bounds begin after HTTP headers are received. Do not expose plain app/PostgreSQL ports publicly.

Build and reproduce the owned synthetic load fixture:

```sh
docker build --target pilot -f deploy/Dockerfile -t wk-hosted-pilot:oct10-owned .
scripts/cargo build -p wk-server
python3 scripts/hosted-pilot.py --duration 30 --output /private/hosted-pilot.json
```

The harness allocates new containers/network/database, five distinct upstream-signed test identities, separate API/collector audiences and installation keys, a reviewer, a scoped admin and restricted services. It uses the actual application runtime grants and real HTTP/signed collector/MCP reads, duplicate idempotent writes and a preassembled offline-catch-up burst. It removes its own containers/network/volumes on exit; the build image/cache remain for reproducibility. It never uses a real project's database. Signing and client generation run outside the app container. This fixture is separate from the real upstream PKCE/browser/macOS Keychain acceptance; its replay burst is not a native disk-queue or cross-client capture certification.

At five developers×50 sanitized10KiB summaries/day, one persistent shared copy adds2.44MiB/day or0.87GiB/year of JSON bodies.30days of active private copies adds73.2MiB before metadata. Revisions, full-text/other indexes, owner deletion markers, credential/nonces/audit, WAL and retained encrypted backups add storage. Four times shared body growth (about3.48GiB/year) is an illustrative allowance, not a measured guarantee; every correction may add another retained revision.50/day×30days is1500 active private notes per owner, below the existing2000 limit. Draft and withdrawn shared rows also consume storage; logical withdrawal does not instantly erase old summary revisions.

A2–4vCPU/8GiB/40GiB starting server is provisional. Choose disk from the measured workload/content density, shared revision/retention policy, WAL/checkpoint behavior and backup count; keep operational free space and track growth. Framework count alone does not determine load. Locally generated repetitive text may compress better than real summaries. No central LLM inference is included in these API/database budgets.
