# Hosted portal operations

Implementation status: scoped administration is locally tested. Physical cleanup, deletion-safe restore and a measured deployment pilot are the remaining work in the approved operations plan. This document does not certify a production identity provider or server capacity.

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
