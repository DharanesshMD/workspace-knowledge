# Scoped Administration and Hosted Operations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the approved portal roadmap with scoped audited administration, bounded physical cleanup, deletion-preserving restores and reproducible five-developer measurements.

**Architecture:** Keep developer UI and owner-only content authorization unchanged. Add explicit product administrator assignments with a bootstrap-defined service ceiling, narrow authenticated administrative methods and metadata-only audit records. Run retention through a separate operator process; preserve independent deletion journals across restores. Measure actual database/API behavior with five synthetic identities and keep deployment claims separate from local measurements.

**Tech Stack:** Rust/Axum/SQLx/PostgreSQL, existing OIDC and Ed25519, shell PostgreSQL/age backup tools, Docker Compose, Python load/restore harnesses.

**Spec:** `docs/architecture/2026-10-10-hosted-developer-portal-plan.md`, sections5 and8–12; approved roadmap increment5. Devices acceptance at9804989 is the input baseline. User approved continuous execution and deferred selection of a production identity provider.

## Global Constraints

- Raw conversations stay encrypted on developer devices. Administrative metadata, logs, journals, backup sidecars and metrics contain no raw messages, credentials or owner-summary text.
- Administrative authority is separate from `can_review` and explicitly scoped by organization/product/service ceiling. It never grants access to another owner's private summary.
- Initial administration remains a CLI; do not create a generic HTTP command runner or remote local-device configuration.
- Identity mappings, administrative role assignment, widened inference ceilings, secrets and TLS remain protected deployment-operator operations. Product administrators manage only already-mapped identities within assigned scope.
- New folders, model endpoints or broader capture still require local approval. Server inference policy may restrict an approved local destination but cannot redirect it or approve a new one.
- Default hosted owner-summary maximum30days, device report maximum7days and content-free audit maximum90days. Admins may shorten these product maxima; published knowledge remains until explicit withdrawal. Physical deletion and backup deletion are different claims.
- Retain installation-key tombstones and deletion idempotency markers beyond the oldest backup. Never remove collector credential bindings before upstream expiry plus120seconds.
- Keep API stopped during restore; remove memberships/admin assignments/browser sessions and revoke installations/workspace bindings before restored access becomes visible. Merge the latest separate shared and personal deletion journals before explicitly granting access again.
- Only disposable `wk_test` and new isolated restore databases/containers are used in tests. Preserve unrelated work and existing client profiles. No deployment, push or merge.
- Production provider, HTTPS proxy, cross-platform native capture and the unresolved RSA advisory remain release gates; this increment cannot certify them indirectly.

## Review Focus

1. A reviewer, guessed product, administrative role removed mid-request, or service ceiling narrowing must not manage peers or expose unpublished/private context; use non-owner runtime RLS tests and authorization locks.
2. Revoking/regranting membership must not resurrect an installation; scope changes and collector writes must lock in a consistent device-before-membership order.
3. Reduced retention, repeated retries and cleanup failure must not extend expired text or delete unexpired credential replay protection; exercise exact cutoff/batch boundaries.
4. Restoring an old snapshot plus a later deletion for a note absent from that snapshot must prevent both body resurrection and idempotent retry recreation; reject partial/nonempty restores.
5. Five concurrent collectors, browser/MCP reads, duplicate retries, offline catch-up and review operations must be measured without claiming local hardware represents a provisioned server; capture errors, latency, memory and database growth.

## File responsibilities

- `migrations/server/0010.sql`, `crates/knowledge/src/admin.rs`, `crates/knowledge/tests/admin.rs`: explicit role/service ceiling, typed product policy and narrow fixed-schema administrative functions. Existing membership RLS remains owner-only.
- `crates/server/src/admin_cli.rs`, `crates/server/src/main.rs`: strict bounded stdin commands; ordinary upstream token verification resolves Actor. Bootstrap role assignment uses the privileged operator connection and no bearer impersonation argument.
- `crates/knowledge/src/lifecycle.rs`, `crates/agent/src/network.rs`: expose effective inference restrictions and apply them before local inference; never change destination.
- `migrations/server/0011.sql`, `crates/knowledge/src/retention.rs`, `crates/knowledge/tests/retention.rs`: personal deletion journal, idempotency guards, bounded cleanup/replay.
- `deploy/scripts/personal-deletions.sh`, `deploy/scripts/restore.sh`, `scripts/hosted-restore-drill.py`: encrypted independent journal export/merge, atomic quarantine and actual isolated restore acceptance.
- `crates/server/src/health.rs`, deployment Compose/runtime configuration and `scripts/hosted-pilot.py`: readiness, bounded process/pool/requests, actual five-identity load and sizing evidence.
- `docs/operations/hosted-portal.md`, `docs/reviews/2026-10-10-hosted-operations-acceptance.md`: actionable operator procedure, measured results, remaining gates and exhaustive rulings.

### Task1: Scoped administration and effective product restrictions

**Interfaces:**
- `AdminAssignment {product:String, user:String, services:Vec<String>}` assigned only by protected bootstrap. The member must already exist and its services must contain the role ceiling.
- `MemberChange {user:String, services:Vec<String>, can_review:bool}`; target identity must already be mapped to the actor's org. Services1–50 valid IDs and subset of administrator ceiling. No command grants administrator status or creates a product/identity.
- `ProductPolicy {private_days:u16, report_days:u16, allow_loopback:bool, https_origins:Vec<String>}` defaults30/7/true/empty. At most16 exact HTTPS origins without userinfo/path/query/fragment. Admin edits may only shorten retention or remove permitted destinations/loopback; widening requires protected bootstrap and an audit event. All relevant membership policy revisions advance. Existing expiries are reduced, never extended.
- `Knowledge::admin_assign(&Actor,&str,&[String])->Result<(),Error>` protected bootstrap only; `admin_members(&Actor,&str)->Result<Vec<MemberMetadata>,Error>` max100; `admin_member_put`, `admin_member_remove`, `admin_policy_put`, `admin_device_revoke`. No content retrieval.
- CLI `wk-server admin` consumes `{token,product,command}` on private bounded stdin; tagged commands `members`, `member-put`, `member-remove`, `policy-put`, `device-revoke`. OIDC verification uses the ordinary production audience and configured issuer, never accepts Actor from input. `wk-server admin-assign` is protected operator bootstrap stdin `{org,user,product,services}`.

- [ ] Write `crates/knowledge/tests/admin.rs` with disposable setup and restricted runtime-role execution. Test reviewer denial, wrong product/org, unmapped target, service ceiling escape, private/unpublished content isolation, administrator role withdrawal, metadata-only audit, scope shrink/device revocation and regrant non-resurrection. Use actual protocol bodies in existing server/knowledge helpers.

```rust
assert_eq!(runtime.admin_member_remove(&reviewer,"demo","alice").await.unwrap_err(),Error::Denied);
runtime.admin_member_remove(&admin,"demo","alice").await.unwrap();
k.grant(&alice,"demo",&["backend".into()],false).await.unwrap();
assert!(matches!(runtime.device_bound(&device,DevicePermission::SharedUpload).policy(&alice,"demo").await,Err(Error::Denied)));
assert!(runtime.personal_list(&admin,"demo",None).await.unwrap().is_empty());
```

- [ ] Run `cargo test -p wk-knowledge --test admin -- --include-ignored` through this plan's disposable DB runner. Expected missing module/method RED; implement role/policy schema and repository.

```sql
CREATE TABLE product_admins (
 org TEXT NOT NULL, product TEXT NOT NULL, user_id TEXT NOT NULL, services TEXT[] NOT NULL,
 PRIMARY KEY(org,product,user_id),
 FOREIGN KEY(org,product,user_id) REFERENCES memberships(org,product,user_id) ON DELETE CASCADE
);
```

Use fixed `SECURITY DEFINER SET search_path=pg_catalog` functions for admin metadata/mutations, revoked PUBLIC execute grants and only required application grants. Check current actor/org against an explicit role and lock that assignment through the operation; services must still be contained by current membership. Do not widen normal memberships SELECT RLS: existing card policies join memberships and would expose peer unpublished records if membership visibility broadened. Lock affected devices by UUID before membership updates/removal; revoke narrowed/revoked installations, increment device revisions, retain key/credential tombstones. Insert content-free action/entity audit in the same transaction. Validate and return opaque errors.

- [ ] Add strict CLI parser/OIDC tests: unknown keys/verbs, oversized input, missing/collector/wrong-audience token, actor spoof and invalid policy. Observe RED before CLI. Add the CLI module and bootstrap dispatch. Secrets never appear in argv/stdout/errors; list responses contain only bounded membership metadata. Verify through a real local-provider test harness rather than claiming production issuer acceptance.
- [ ] Extend effective `policy` with typed restrictions. Validate the already approved inference origin before any request; defaults allow literal-loopback local inference only. Network regression must reject unapproved HTTPS and allow configured exact origins without redirect/destination changes. Observe RED before enforcing restrictions.
- [ ] Run all Rust DB suites plus existing portal smoke, fmt/clippy. Expected PASS and no regression in owner/team/device access. Commit `feat: add scoped audited portal administration`.

### Task2: Physical retention and personal deletion journal

**Interfaces:**
- `personal_deletions(org,product,owner,note_id,revision,key_hash,deleted_at)` retains no body or raw idempotency key; unique scope/key hash and scope/note identity. No product FK because post-snapshot deletions can reference a product absent from the backup.
- Trigger on transition to deleted journals SHA256 of UTF8 idempotency key in the same transaction, SECURITY DEFINER with fixed schema; backfill existing deleted rows. No application direct journal mutation or journal content route.
- `Knowledge::retention_batch(limit:u32)->Result<RetentionCounts,Error>` deployment-operator only; limit1–1000, one advisory worker lock, bounded ordered `FOR UPDATE SKIP LOCKED` selection per table. Counts only; no bodies/IDs. Clear expired personal bodies, set deleted/revision, preserve marker and journal; delete expired reports/activity/browser sessions/nonces/credential hashes and audit rows older90days. Unexpired collector hashes and installation tombstones remain.
- `Knowledge::replay_personal_deletions()->Result<u64,Error>` operator only. Match exact org/product/owner/note-ID or key hash, clear body and set deleted, preserve maximum revision. `personal_put` rejects journaled keys even if no note row exists in the restored DB.
- CLI `wk-server retention-once` and `retention-worker`; worker runs batches every60seconds, handles failures with content-free status and bounded delay, exits on shutdown. It uses a separate protected operator connection/process; no cleanup HTTP route or app cleanup privilege.

- [ ] Write exact boundary tests before implementation: expiry-minus1 stays, expiry clears text and journals; retries cannot renew a deleted key; orphan post-snapshot key journal prevents recreation; unrelated owner/product unaffected; second batch idempotent; one bounded batch processes only configured count; runtime role cannot invoke physical cleanup/direct journal writes. Observe missing module/journal RED.

```rust
k.personal_delete(&alice,"demo",note.id,1).await.unwrap();
assert_eq!(journal_body_fields(&pool).await,0);
remove_only_synthetic_note_row(&pool,note.id).await;
assert_eq!(k.personal_put(&alice,"demo",input("one")).await.unwrap_err(),Error::Conflict);
let counts=k.retention_batch(1).await.unwrap();
assert!(counts.personal<=1);
```

Implement the new migration, fixed-schema deletion trigger and operator cleanup/replay. Use the existing personal owner advisory lock before checking deletion hashes; membership still guards every owner operation. Do not extend creation-relative TTL on edits, retries or policy changes. Require proper operator DB permissions rather than trusting a caller-supplied Actor.
- [ ] Test worker shutdown/error behavior and audit age. Run full Rust DB suites and fmt/clippy. Commit `feat: preserve personal deletions and bound hosted retention`.

### Task3: Encrypted deletion-safe backup and restore acceptance

**Interfaces:**
- `personal-deletions.sh export AGE_RECIPIENT OUTPUT.age` atomic no-overwrite publication from private temp directory; `merge AGE_IDENTITY INPUT.age` fixed COPY columns/types in one transaction. Protected PostgreSQL service credentials, strict shell options, no SQL interpolation of decrypted fields. Merge max revision/date and replay matching bodies before commit.
- Restore stops all client access: restored memberships/admin assignments/browser sessions removed; installations and workspace bindings revoked; latest shared/personal journals required before reprovisioning. New migration version expectations explicit; refuse incompatible archives and nonempty target without partially restoring.
- `scripts/hosted-restore-drill.py` allocates owned synthetic DBs, keypair and fixtures itself. Source contains shared published/private notes, device/browser/admin metadata. Backup precedes explicit deletions; also create/delete a private note after snapshot to prove orphan-key protection. Restore target is independent and empty.

- [ ] Run new acceptance against original restore/journal tools, expected RED: personal journal command missing or restored browser/device authority persists.
- [ ] Implement encrypted personal journal tool, update restore atomic quarantine and operator instructions. Use fixed COPY temp files and exact identity/scope matching. Test malformed/tampered ledger and interrupted/nonempty restore failure without enabling access.
- [ ] Actual encrypted restore must prove: memberships/admin/browser all absent, old installation permanently revoked even after explicit regrant, shared withdrawal replay removes retrieval, private text cleared, orphan retry denied, later journal re-merge idempotent, unrelated note preserved, nonempty restore refused. No real project DB used. Record versions and outputs. Full suites green, commit `feat: quarantine restored access and replay personal deletions`.

### Task4: Deployment bounds and measured five-developer pilot

**Interfaces:**
- `/health/live` returns only `{live:true}`; `/health/ready` checks a bounded DB connection and exact schema version and returns no credentials/config/counts. Production behind existing same-origin HTTPS proxy.
- Explicit pool/concurrency/timeouts with bounded validated configuration: default app pool10, acquire5seconds, SQL statement5seconds/lock3seconds, HTTP30seconds, at most64 in-flight API requests; overload returns an opaque503 rather than indefinite queues. No infinite retry loop.
- Compose app/PostgreSQL CPU and memory limits, healthcheck/readiness, existing non-root/read-only/cap-drop/private DB isolation. Initial pilot budgets are configurable and not performance promises. Secret values remain operator-managed.
- `scripts/hosted-pilot.py` uses five distinct synthetic accounts/keys with reviewer, admin and restricted-service coverage; runs actual signed collectors, owner/team/search and ordinary MCP access, duplicate retries, offline catch-up burst and review writes. No central inference or raw transcripts.

- [ ] Write health/invalid-config/overload tests first; observe404/unbounded behavior RED. Implement bounds and update deploy docs/Compose. Do not run readiness through an owner connection or expose DB diagnostics.
- [ ] Pilot warm-up then fixed documented duration; measure p50/p95 latency by operation, request/error totals, effective throughput, process RSS, container CPU/RAM, DB/pool observations, database/table/index bytes before/after and backup bytes. Record request schedules, actual hardware, all resource caps, payload size, nonce-window pressure and sampled bottlenecks. Explain extrapolation assumptions separately from measurements.
- [ ] Include five actors ×50 sanitized summaries/day ×10KiB planning comparison, retained private30day versus persistent shared growth, indexes/WAL/backup allowance. Report measured density rather than treating2–4vCPU/8GB/40GB as proven requirements. If Linux app container cannot be built/run, report local pilot limitation explicitly; do not imply server acceptance.
- [ ] Run complete Rust/DB and browser suites, build, fmt/clippy, audit, live OIDC device/admin, cleanup, encrypted restore and pilot checks. Write durable operations acceptance and architecture status. Commit; one fresh whole-increment review, one RED→GREEN Important/Critical fix pass, exhaustive rulings before scratch cleanup. Preserve the feature worktree pending explicit integration instructions.
