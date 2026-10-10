# Hosted operations acceptance — 2026-10-10

Status: implementation and local acceptance are complete through deployment bounds; final independent review is pending. **Production rollout is not certified.** The provider is deliberately a local test provider until the user selects a production issuer. Known RSA advisory and deployment-specific HTTPS/identity/native/capacity gates remain open.

## Implemented boundaries

Scoped product/service administration is separate from review permission and never grants peer-private or unpublished-summary access. Identity mapping, administrator assignment and widened policy remain protected operator bootstrap. Administration is initially a typed authenticated CLI, not a hosted administrative form or generic command runner. Current policies restrict local approvals and never redirect inference; the existing summarizer remains literal-loopback HTTP only.

Retention is a separate bounded operator worker. Private summaries expire logically at creation-relative maximum30days, reports7days and audit90days; policy may shorten them. Physical cleanup clears current private bodies and preserves content-free deletion/idempotency/key markers. Metadata markers remain indefinitely until a backup/replay horizon is defined. Neither SQL cleanup nor logical withdrawal immediately erases historical WAL/pages/backups or previously delivered information.

Encrypted version2 backups include sanitized private summaries and shared data, never local raw stores. Restore into an empty isolated schema12 database atomically removes browser sessions/admins/memberships, revokes devices/workspace bindings, denies PUBLIC helper execution and advances collector authentication epoch. Merge both separately retained newest journals before reprovisioning. Post-snapshot private deletion hashes prevent retry recreation even when the original note was absent. Old/missing-iat collector tokens cannot enroll unseen installations; require issuer iat support, wait past restore+61seconds, then fresh authentication/consent.

Runtime defaults: pool10/acquisition5seconds, SQL statement5seconds/lock3seconds, HTTP30seconds, at most64 in-flight API requests. Overload/deadline failures return opaque503. Slots cover bounded8MiB JSON body completion. Readiness checks exact migrations1–12 through a restricted role, one probe at a time under a1-second deadline. App container is UID10001, read-only, cap-drop/no-new-privileges; app/database ports remain private behind the intended same-origin HTTPS proxy.

## Verification evidence

- Rust workspace PostgreSQL suites:230 passing,0 failed/ignored; latest separate bounds4/4 include exact-schema restricted-role readiness, future-schema refusal, opaque failures, invalid limits, overload, slot release and actual SQL statement/lock/pool deadlines. Final task-done suite remains to be recorded with the commit.
- Browser:30/30 plus TypeScript/Vite production build. Refreshed real desktop/mobile device screenshots were visually inspected; no horizontal overflow or invented capture success.
- Real local upstream public PKCE/browser/OIDC admin CLI: explicit role, reviewer/product/service denial, private-summary isolation, policy/new-note TTL, revocation/logout and CSRF/origin/nonce/signature rejection passed.
- Real macOS native public PKCE/OS Keychain: login, expiry reauthentication, requested/applied pause/resume, local pause precedence, opt-in synchronization, owner/device privacy, signed policy, replay/audience escape, revocation and fresh-credential replacement passed. One initial run timed out at the unchanged3-second transport deadline during simultaneous cold builds/full DB suite; isolated rerun passed. This is not system-browser dispatch, universal capture, Windows/Linux keyring or restored-native reconnection certification.
- Actual age1.3.2/PostgreSQL17.11 encrypted restore:13 acceptance checks passed, including fresh migrations, quarantine, PUBLIC denial, both latest journals, orphan-key guard, repeat merge, unrelated-note preservation, nonempty refusal, invalid header/rows, tampered ciphertext, old-manifest refusal and rollback after injected pre-commit failure. Initial idle drill5.87seconds; cold-build contention drill93.98seconds demonstrates why neither is a production RTO.
- Rust fmt/clippy and Compose configuration pass. Linux aarch64 release image/console/server build passed. Dependency audit fails on the known `RUSTSEC-2023-0071`, `rsa0.9.10`, through normal-runtime `openidconnect` in server/agent. No audit exception or release clearance was added.

## Five-developer measurements

[Complete metadata/count/timing dataset](hosted-operations/pilot.json). App image `sha256:ab55add8fd8452e69204bf8e88368b6cb4baa9523440ec2162e50f47a2d3ef28`; PostgreSQL17.11 aarch64 Alpine. Mac host8logical CPUs/16GiB RAM; Docker VM8CPUs/about11.67GiB. Linux app and database each capped2CPU; app1GiB, database2GiB. No central inference was included.

Five distinct accounts/installation keys, a reviewer, scoped administrator and restricted backend/frontend services. Five client threads issue one HTTP request at a time each, with local OpenSSL signing and50ms cycle pauses. Each normal cycle has10 requests: signed policy, private write/retry, shared draft, review publication, owner/team/search/MCP reads and review queue. Warmup5.004seconds/50 cycles; measured steady30.894seconds/208 cycles;100 preassembled queued summaries then replayed/published in4.056seconds. Private JSON bodies10,240bytes, shared10,247bytes. Queued inputs model reconnect load; they do not prove native disk-queue replay. JWT verification is real, but issuance in this load fixture is synthetic and separate from upstream-PKCE acceptance.

**2,380 measured requests,0 errors,68.1 requests/second** across steady+burst. Initial independent run2,560 requests/0errors at73.42requests/second; final run corrected CPU timestamp sampling. This is a short local pilot, not saturation, long-duration soak, real-content cardinality or remote-server acceptance.

| Operation | Requests | p50 ms | p95 ms | Maximum ms |
|---|---:|---:|---:|---:|
| collector policy | 208 | 30.53 | 181.97 | 1012.35 |
| collector personal | 516 | 38.4 | 168.13 | 608.18 |
| collector cards | 308 | 40.39 | 123.26 | 391.94 |
| publish | 308 | 25.2 | 100.06 | 373.23 |
| owner | 208 | 23.07 | 92.76 | 226.56 |
| team | 208 | 37.27 | 109.42 | 853.63 |
| search | 208 | 26.19 | 597.7 | 844.92 |
| mcp | 208 | 24.76 | 599.66 | 921.71 |
| review | 208 | 9.64 | 52.23 | 413.45 |

App mean CPU0.366cores/approximate sampled peak0.523cores; database mean1.165cores and sampled peak about its2-core cap. PostgreSQL recorded14,500,691microseconds of aggregated runnable-task throttling, consistent with CPU pressure and search/MCP p95 around600ms. Throttle time is not wall-clock outage. Peaks use per-container guest uptime beside cgroup counters; scheduling/quota windows make individual intervals approximate.

Peak cgroup memory: app8.74MiB/database171.67MiB. PID1 RSS app15.61MiB/PostgreSQL parent32.15MiB; cgroup memory includes workers/cache, while mapped shared pages can appear in RSS without being charged to that group. These are different accounting measures, not forecasts for large datasets. Sampled runtime pool:10 connections, at most4 active,0 observed lock waits. Pool acquisition wait duration is not independently instrumented, and sparse samples cannot prove absence of waits. Maximum261 nonces per device stayed below the1200/120-second window; no nonce saturation claim.

DB allocation after warmup10,385,075bytes →15,324,851bytes; growth4,939,776bytes across308 additional private/shared pairs (about15.66KiB/pair including revisions/graphs/indexes/operational rows). WAL counter grew5,934,816bytes; allocated WAL segment space can be larger. Final encrypted full snapshot1,188,328bytes. One actual operator retention sweep cleared/journaled one expired private body. Fixture text repeats technical vocabulary and compresses well; this density must not be extrapolated to arbitrary developer summaries.

Owned pilot containers/network/anonymous volumes and failed-test fixture database/role were removed; build image/cache remain for reproducibility. The original project database and native client profiles were not used.

## Storage planning and remaining release gates

Five developers×50 summaries/day×10KiB: persistent shared bodies2.44MiB/day or0.87GiB/year, plus active private30-day bodies73.2MiB. Indexes, shared revisions (including drafts/withdrawn history), indefinite metadata and WAL add storage. A4×shared-body allowance is about3.48GiB/year, explicitly illustrative. Multiple full backups multiply archive storage; use a separate protected backup destination or include their count in disk sizing.50/day×30days is1500 active private notes per owner, below the existing2000 cap.

A2–4vCPU/8GiB/40GiB starting allocation remains provisional. The measured database CPU pressure calls for a deployment-specific test before selecting CPU;40GiB cannot be promised sufficient for arbitrary archive duration/content/revisions/backups. Framework count alone does not determine traffic, and local LLM compute is excluded.

Release gates: chosen production issuer/native client registrations and iat behavior; same-origin HTTPS/proxy connection/header/body deadlines and rate limits; secret/vault/operator/journal recovery procedures; RSA advisory resolution or reviewed mitigation; real restored-native reauthentication after cooldown; Linux/Windows client and wider version-specific capture coverage; target-server load/soak/storage acceptance. Native Codex/Antigravity and other clients retain their existing bounded acceptance/blocked-version facts; this portal adds no universal capture claim.

## Rulings I made

- Ruling: execute this detailed operations plan under the already approved architecture roadmap and continuous proceed instructions — no additional deployment or integration is authorized — cost if wrong: reversible local plan/implementation changes.
- Ruling: initial administration uses explicit product/service-scoped assignments and narrow authenticated CLI, while identity mapping and role/inference-ceiling widening remain operator bootstrap — preserves existing privileged provisioning boundary without a generic admin command runner — cost: organization-wide administration requires explicit assignments per product and protected operator access for bootstrap changes.
- Ruling: a partial-service administrator cannot change a product-wide restriction or manage a member/device whose full scope exceeds its ceiling — product policy affects all members, so require authority over every current product service — cost: broader product policy edits require a fully scoped administrator.
- Task1 Ruling: retain local-only inference in the existing summarizer even when operator policy names HTTPS origins — product policy can restrict an approved destination but cannot expand the collector's transport/capture approval — cost: HTTPS inference remains unsupported until separately authorized local implementation and privacy verification.
- Task1 Ruling: keep legacy unsigned loopback fixtures on default local inference policy, but require explicit restrictions on hosted signed policy — backward compatibility must not silently bypass deployed restrictions — cost: hosted collectors require upgraded server schema/projection.
- Task1 Ruling: protect privileged bootstrap scope changes with the same device-before-membership locking and tombstone revocation as scoped admin changes — regrant must not restore an old installation or narrowed admin assignment — cost: privileged provisioning now expects migration10 and can contend with in-flight device operations.
- Task2 Ruling: physical cleanup uses a separate protected table-owner/operator connection and no application-role function grant — administrative product authority cannot erase global rows — cost: deploy an independent operator worker and protect that credential separately from app credentials.
- Task2 Ruling: keep deletion journals, personal idempotency markers and installation-key tombstones indefinitely in this initial increment — newest journals must cover every retained backup and unknown replay horizon — cost: bounded content is removed but metadata storage still grows and requires a later measured archival policy.
- Task3 preflight Ruling: restoring a snapshot also needs a collector authentication cutoff — an unseen installation/token issued after the snapshot is absent from its binding table and otherwise can re-enroll after membership reprovisioning — cost: restored deployments require fresh upstream collector authentication and an issued-at claim; provider compatibility is a release gate. Add a protected global collector epoch, enforce it only in collector auth and advance it atomically during restore. Ordinary independent user API/MCP credentials remain separate.
- Task3 Ruling: preserve a60-second upstream clock-skew allowance and set restore collector epoch to restore time+61seconds — prevents previously accepted future-issued tokens from crossing the restore boundary — cost: require cooldown and an issuer iat claim; fresh-sign-in test models elapsed cooldown in its isolated fixture rather than live61-second native reconnection.
- Task3 Ruling: explicitly revoke PUBLIC function/schema access after restoring without dump privileges — omitted ACLs otherwise recreate SECURITY DEFINER helpers with PostgreSQL PUBLIC defaults — cost: operators must reapply the exact restricted runtime grants before serving.
- Task3 Ruling: collector repository storage failures return temporary503 instead of authorization401 — a database outage must not look like revoked consent and clear held native drafts — cost: callers retry transient failures within existing bounded behavior.
- Task3 Ruling: support only v2 archives with exact schema12 for this restore path — mixed-version access/deletion tables cannot be safely inferred — cost: older snapshots require a version-specific offline upgrade, never manifest rewriting.
- Task4 Ruling: buffer API and stateless MCP JSON responses up to8MiB under the same request deadline and slot — returning headers alone must not release an in-flight request while a body keeps working — cost: future streaming/SSE support needs a separately bounded design; current50-row/64KiB summaries fit below the cap.
- Task4 Ruling: liveness/readiness remain outside the ordinary request gate, with readiness restricted to one concurrent query and a1-second deadline — overloaded clients must not queue unlimited readiness queries or suppress basic process liveness — cost: concurrent readiness probes may get temporary503.
- Task4 Ruling: measure a disposable Linux app fixture with actual JWT verification/signed collector and restricted runtime database role, but leave production issuer/TLS/browser/native queue acceptance separate — local hardware and synthetic replay cannot certify remote deployment — cost: a deployment-specific acceptance/load run remains necessary.

## Independent review

Pending one fresh whole-operations review and one Important/Critical fix pass.

## Deferred minors

None recorded before the independent review.
