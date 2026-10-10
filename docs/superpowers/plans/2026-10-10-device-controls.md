# Device Setup and Controls Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect locally approved collectors to the hosted portal with enforced device credentials, private bounded health, and honest pause/revoke acknowledgments.

**Architecture:** Keep upstream OIDC as the issuer. A separate collector audience is accepted only on a narrow `/collector` API, alongside proof of possession of an installation key. A registered device is scoped to one owner/product, approved services and local approval hash. Ed25519 signatures use the existing library and a versioned, unambiguous request envelope; they are an application device binding, not an OAuth DPoP implementation. Database operations hold a device row lock through writes, so revocation and uploads have a defined order. Browser device controls use existing cookie/CSRF authorization and owner-only RLS.

**Tech Stack:** Rust/Axum/PostgreSQL, existing Ed25519 library, upstream OIDC/PKCE, native OS credential store, React/Playwright.

**Spec:** `docs/architecture/2026-10-10-hosted-developer-portal-plan.md`, sections 5–8 and 11–12. This is the fourth approved roadmap increment; production identity provider remains user-selected later.

## Global Constraints

- Never send raw messages, native session IDs, absolute paths, arbitrary error strings, credentials or serialized local configuration in health reports.
- New folders, broader capture, a new inference destination or credential changes require local confirmation.
- Device revocation blocks future collector uploads at the server. Local pause remains pending until observed by the device. Independent browser/MCP user credentials are not device credentials.
- Owner sync and activity each require both local consent and remote permission. Defaults off for a newly registered device; remote resume cannot cancel a local pause.
- Do not create an OAuth issuer, accept collector tokens through ordinary API/MCP, add a generic remote command runner, deploy, push or merge.
- Preserve explicit workspace boundaries and version-specific native coverage. An installed hook or connected collector is not proof of complete capture.
- Literal-loopback development tests may use separate synthetic collector tokens. Production requires a distinct configured collector OIDC audience and public client. Reject tokens whose audience includes both roles.

## Review Focus

1. A revoked key/token attempts a regular API or MCP route, or tries to register the same installation again: denied, no upload escape.
2. Revocation or pause races with a write: row locks order operations; an acknowledged revocation prevents later writes.
3. Device state/config changes during inference: old destination, approval or service cannot inherit renewed consent.
4. Offline collectors, local pause and stale acknowledgments: UI must show requested and applied separately and avoid a misleading green state.
5. Guessed device IDs, changed service membership and unsafe health fields: no peer/reviewer metadata access, safe opaque failures, bounded reports.

### Task 1: Scoped device repository and signed collector API

**Files:** New `migrations/server/0008.sql`, `crates/knowledge/src/devices.rs`, `crates/knowledge/tests/devices.rs`, `crates/contracts/src/device.rs`, `crates/server/src/device_routes.rs`, `crates/server/tests/devices.rs`. Modify knowledge migration/actor transaction, selected collector write/read methods, server router/auth configuration, runtime grants.

**Interfaces:**
- `DeviceRegistration {id:Uuid, public_key:String(hex32), product:String, services:Vec<String>, approval_hash:String(hex32)}`; public browser response omits public key and approval hash.
- `DeviceSettings {paused:bool, owner_sync:bool, activity:bool}`; `DeviceChange {expected_revision:i64, settings:DeviceSettings}`. Owner list is product scoped; revoke uses expected revision and is irreversible for that installation key.
- `DeviceReport {collector_version:String, config_revision:i64, config_status:applied|rejected, local_pause:bool, local_owner_sync:bool, local_activity:bool, queue:empty|small|large, reason:allowlisted enum, capabilities:Vec<Capability>}`. Capability client/surface/status enums; installed version is at most64 ASCII version characters beginning with a digit. At most15 capabilities, compact report4KiB, one latest report with seven-day expiry. Reports are collector-reported facts, not server-certified capture.
- `DeviceGuard {id,product,services,approval_hash,permission:read|shared_upload|owner_upload|activity_upload}` clones Knowledge per request. `actor_tx` validates the current owner/device/membership, locks device and membership through mutation, and checks current remote restrictions. Guard-scope checks reject wider services; overlaps uses only approved device services.
- Versioned signed envelope is JSON array `["wk-device-v1", origin, method, path, timestamp, nonceUUID, deviceUUID, SHA256(token), SHA256(bodyBytes)]`. Request headers hold ID/time/nonce/signature only; replay validity60s and retained nonce120s. Nonce use and bounded request rate are enforced in PostgreSQL, not process memory.
- `/collector/register`, `/collector/policy`, `/collector/cards`, `/collector/personal`, `/collector/activity`, `/collector/overlaps`, `/collector/report` are narrow POST routes. No browser cookies/origin, generic commands, arbitrary URL/query or admin operations. Production collector audience is separate from ordinary audience. Browser GET/PATCH/POST owner device routes are `/v1/products/{product}/devices`, `.../{id}/settings`, `.../{id}/revoke`.

- [ ] Write DB tests for owner/reviewer/cross-org/service isolation, registration idempotency, key revocation non-resurrection, stale settings, strict unsafe-report rejection, expiry, remote pause/local opt-ins and actual guarded writes. Cover restricted non-owner runtime RLS and a deterministic write/revoke ordering case.

```rust
let guarded = k.device_bound(&device, DevicePermission::OwnerUpload);
k.device_revoke(&owner,"demo",device.id,device.revision).await.unwrap();
assert!(matches!(guarded.personal_put(&owner,"demo",input).await,Err(Error::Denied)));
```

- [ ] Run disposable DB tests before implementation: expected missing device module/method failure. Implement tables with owner-only current-membership RLS, versioned settings, unique installation key and bounded per-owner capacity. Add guard checks at actor transactions and scope checks for the four exposed repository operations. Grant only needed runtime operations. Run all server/knowledge tests including ignored disposable DB cases; expected PASS.
- [ ] Write HTTP tests: no collector credentials401; ordinary token/cookie rejected; missing/wrong/replayed proof denied; body/path tampering denied; revoked registration cannot reactivate; audience overlap rejected; unsafe payload generic422/no echo. Run before routes: expected404. Implement separate collector verifier/tokens and strict signed route dispatch using current device binding. Run targeted plus existing HTTP suites; expected PASS. Commit.

### Task 2: Local collector connection and effective controls

**Files:** New agent `device.rs` and native-login module; modify state/main/network/daemon/personal-sync/team; tests stay in focused Rust modules.

**Interfaces:** CLI `device connect` uses an approved local binding, system-browser PKCE public-client sign-in, OS-stored upstream credential and signing key. Remote configuration publishes issuer/client ID and fixed issuer endpoints, never secrets. Local state stores only public registration/config receipt. Every production collector network call uses the collector namespace and signs exact bytes. Existing loopback legacy fixtures remain explicitly development only.

- [ ] Write tests for signed envelope tampering, URL-bound key use, safe telemetry, local-and-remote privacy intersection, pause acknowledgment, changed approval/destination, and production legacy refusal. RED expected missing functions; implement request signing and native PKCE with transaction state, nonce, exact ephemeral loopback callback, fixed issuer/bounded no-redirect network, no secret logging. Persist credentials only through native keyring. Refresh upstream credential safely or show auth-required; never silently widen audience.
- [ ] Poll/report before processing; apply only allowlisted settings. Use current local binding and local consent; remote pause stops capture/processing and still permits bounded health/ack delivery. Retention loop stays independent. Status exposes requested/applied/rejected, not fabricated presence. Run all agent tests, formatting/clippy and synthetic signed requests; expected PASS. Commit.

```rust
assert!(!effective_owner_sync(local_consent_false, remote_enabled_true));
assert!(effective_pause(local_pause_true, remote_pause_false));
```

### Task 3: Hosted Devices and setup UI

**Files:** New `web/src/Devices.tsx`, `web/tests/devices.spec.ts`; modify App/types/styles and setup documentation.

- [ ] Write browser tests for owner-scoped empty/list state, no transcripts/paths/secrets, honest stale/unknown capability states, expected-revision pause/revoke, conflict reload, delayed product response cancellation, keyboard controls and390px overflow. RED expected missing Devices view.
- [ ] Implement product-keyed abortable Devices screen. Show last reported time, requested settings, acknowledged settings and local consent separately. Fresh report does not imply capture supported. Provide local CLI setup instructions with placeholders for local path arguments; no credential in command and no browser-to-localhost bridge. Revoke requires in-page confirmation with clear future-upload effect. Run build/all browser suite; inspect desktop/mobile captures. Expected PASS. Commit.

```ts
await page.getByRole('button',{name:'Devices',exact:true}).click();
await expect(page.getByText('No registered devices in this product.')).toBeVisible();
```

### Task 4: Real local-provider device acceptance and final review

**Files:** Add a synthetic device smoke harness and live browser script; acceptance report under `docs/reviews/2026-10-10-device-controls-acceptance.md`; update architecture status.

- [ ] Exercise real upstream test-provider collector sign-in, proof requests, two identities and service scopes; owner upload opt-in, explicit report/ack, pause/offline/resume, revoke then attempted upload/replay/ordinary API escape. Real browser uses live API, no mocks. Use only disposable wk_test; document native keyring/native-browser and chosen-provider limitations separately if not verifiable.
- [ ] Run Rust suites with DB cases, web build/tests, fmt/clippy and whitespace. Record exact counts/statuses. Commit, one fresh independent review of this increment, one RED→GREEN pass for Important/Critical findings. Preserve every ruling before scratch cleanup. Continue scoped Admin/operations pilot roadmap; no deployment claim.
