# Hosted Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver hosted browser sign-in, scoped product selection and logout as the first working increment of the approved developer portal.

**Architecture:** The existing Rust server hosts an OIDC relying party and opaque browser sessions. PostgreSQL persists session hashes and fixed deadlines; API cookie authentication resolves the same Actor used by bearer clients. React obtains identity/product membership from `/v1/me`, while MCP remains bearer-only.

**Tech Stack:** Rust/Axum, openidconnect 4, SQLx/PostgreSQL, React/TypeScript/Vite, Playwright.

**Spec:** `docs/architecture/2026-10-10-hosted-developer-portal-plan.md`, sections 3, 6, 8 and increment 1 in section 11. User approved execution and selected a local test provider on 2026-10-10; other portal increments follow separate plans.

## Global Constraints

- One organization per deployment initially; multiple products and services.
- Every request checks current membership.
- Browser cookies are not accepted as MCP credentials.
- Production removes the token-paste entry point.
- Cookies: Secure, HttpOnly, SameSite; proposed 30-minute idle / 12-hour absolute lifetime.
- Cookie-authenticated mutations need CSRF tokens and strict origin checks.
- Authenticated content uses `Cache-Control: no-store`.
- No raw-transcript endpoint, no credential logging, no browser token storage.
- Production OIDC issuer and endpoints require HTTPS; explicit development mode permits literal loopback HTTP for the synthetic provider only.

## Review Focus

- Callback replay or browser-swapped state: consume a short-lived browser-bound transaction once before token exchange (Task 2).
- Account remapping or membership revocation during a session: resolve trusted identity again and load products freshly on every `/me` request (Tasks 1, 3).
- Mixed bearer/cookie authentication: reject ambiguity; MCP remains unaffected by cookie state (Task 3).
- Browser product switch with outstanding requests: remount product content and abort stale requests (Task 4).
- Expired signing keys or provider outage: new authentication fails safely; no user-controlled discovery, redirect or key fetch (Task 2).

---

### Task 1: Persistent browser-session and membership repository

**Files:** Create `crates/knowledge/src/browser.rs`, `migrations/server/0006.sql`, `crates/knowledge/tests/browser.rs`; modify `crates/knowledge/src/lib.rs`.

**Interfaces:** Produces `BrowserSession { issuer: String, subject: String, csrf_hash: String }`; `Knowledge::create_browser_session(hash: &str, issuer: &str, subject: &str, csrf_hash: &str) -> Result<(), Error>`, `browser_session(hash: &str) -> Result<BrowserSession, Error>`, `delete_browser_session(hash: &str) -> Result<(), Error>`, `products_for(&Actor) -> Result<Vec<lifecycle::Policy>, Error>`.

- [ ] Write the ignored disposable-database regression `session_lookup_enforces_expiry_and_current_identity`: provision identity and product, create a session with synthetic 64-character hashes, read it, remap identity, verify fresh identity lookup changes, expire the database row and verify denial. Also assert products are organization/user-filtered and deletion invalidates lookup.

```rust
assert_eq!(k.products_for(&owner).await.unwrap().len(), 1);
k.create_browser_session(&"a".repeat(64), "https://issuer.example", "subject", &"b".repeat(64)).await.unwrap();
let session = k.browser_session(&"a".repeat(64)).await.unwrap();
assert_eq!(session.subject, "subject");
k.delete_browser_session(&"a".repeat(64)).await.unwrap();
assert!(k.browser_session(&"a".repeat(64)).await.is_err());
```

- [ ] Run `scripts/cargo test -p wk-knowledge --test browser -- --ignored`; expected unresolved methods/module initially.
- [ ] Add `browser_sessions(session_hash,issuer,subject,csrf_hash,created_at,last_seen,expires_at)` with session/CSRF hashes checked as 64 lowercase hex characters, a fixed 12-hour expiry, and identity foreign key. Select/update only if absolute deadline is future and last_seen is within 30 minutes; expired lookup cannot refresh itself. `products_for` runs under `actor_tx`, selects only matching memberships and returns existing Policy objects ordered by product. Register migration 6.
- [ ] Re-run database regression, expected PASS; commit only Task 1 files.

### Task 2: Bounded OIDC login transactions

**Files:** Create `crates/server/src/browser_auth.rs`; modify server Cargo dependencies and lockfile; tests in module.

**Interfaces:** Produces `BrowserConfig::load(origin, issuer, client_id, client_secret, development) -> Result<BrowserAuth,String>` async; `BrowserAuth::begin() -> Result<LoginStart,String>`; `complete(transaction_cookie,state,code) -> Result<(String,String),String>` async. `LoginStart { url:String, transaction_cookie:String }`.

- [ ] Write tests for HTTPS/loopback configuration validation and transaction consumption: missing/wrong cookie cannot consume another browser's state; exact state/cookie can be consumed once; expired transaction is rejected.

```rust
assert!(validated_origin("http://example.org", false).is_err());
assert!(validated_origin("http://127.0.0.1:4318", true).is_ok());
assert!(validated_origin("https://user:secret@example.org", false).is_err());
```

- [ ] Run `scripts/cargo test -p wk-server browser_auth`; expected undefined validation/transaction methods.
- [ ] Use maintained `openidconnect` for discovery, authorization code + PKCE S256, nonce/ID-token verification and access-token-hash validation. Only operator-configured issuer discovery; reqwest has no redirects, no proxy and a 10-second timeout. Validate discovered endpoints against HTTPS policy and expected issuer. Retain no upstream credentials after identity extraction. Login transactions have 5-minute deadlines and an in-memory cap of 1,000, hold random browser cookie binding/state/nonce/verifier, and are atomically consumed before network waits. Refresh configured provider metadata/JWKS for each new begin using a bounded scheduled snapshot; fail new login if snapshot is older than five minutes. No caller-provided return URL; success redirects `/`.
- [ ] Re-run module tests, expected PASS; commit Task 2.

### Task 3: Browser routes, cookie authentication and CSRF

**Files:** Create `crates/server/src/browser_routes.rs`, `crates/server/tests/browser.rs`; modify `crates/server/src/lib.rs`, `main.rs`.

**Interfaces:** Consumes Tasks 1/2; produces `/auth/config`, `/auth/login`, `/auth/callback`, `POST /auth/logout`, `GET /v1/me`. Add `portal_router(knowledge,tokens,oidc,origin,browser_auth)` while existing configured_router calls it with no browser provider. Shared `actor` supports cookie identity after browser middleware validates mutation origin+CSRF; existing bearer path remains.

- [ ] Write HTTP tests: `/auth/config` says token-paste is allowed only when no production OIDC verifier; login unavailable without configured provider; `/v1/me` rejects absent identity; a session cookie cannot authenticate MCP; forbidden-origin/missing-CSRF mutation returns 403; authenticated headers include no-store. Database HTTP tests cover logout invalidation and changed identity/membership.

```rust
let response = app.oneshot(Request::builder().uri("/v1/me").body(Body::empty()).unwrap()).await.unwrap();
assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
```

- [ ] Run `scripts/cargo test -p wk-server --test browser`; expected missing route assertions fail.
- [ ] Add a `__Host-wk_session` random cookie in production and separate explicitly named development cookie on loopback; session IDs are 32 random bytes, database stores SHA256 only. Transaction cookies expire after 300 seconds. On successful callback verify provisioned identity before session creation; ignore untrusted display-name/org claims. `/me` returns user/org, fresh product policies and session CSRF token derived from an independent random cookie-secret mechanism held server-side; no access/ID tokens. Middleware rejects ambiguous auth and cookie mutations lacking exact Origin and matching hashed CSRF. Cookie and transaction errors are generic and never echo input. Logout deletes the session and expires cookie, no redirect to arbitrary provider URL.
- [ ] Wire optional `WK_BROWSER_CLIENT_ID` / `WK_BROWSER_CLIENT_SECRET` at startup with fixed `WK_OIDC_ISSUER` and `WK_CONSOLE_ORIGIN`. Development requires explicit `WK_BROWSER_TEST_ISSUER` and exact loopback origin. Missing production browser configuration renders a setup-needed screen, not bearer-token form.
- [ ] Run HTTP tests and all existing server tests, expected PASS; commit Task 3.

### Task 4: Hosted sign-in and product-selection UI

**Files:** Create `web/src/Session.tsx`, `web/tests/session.spec.ts`; modify `web/src/App.tsx`, `api.ts`, `types.ts`, `style.css`, Vite proxy.

**Interfaces:** `/auth/config -> { browser_login:bool, development:bool }`; `/v1/me -> { user,org,products:Policy[],csrf_token?:string }`. `request` sends same-origin cookies when token is empty and X-CSRF-Token on mutations; development token path remains memory-only. React Session passes current product and empty token to existing knowledge workbench.

- [ ] Write Playwright tests: configured anonymous portal shows organization sign-in without token field; signed-in portal shows only permitted products; switching product clears old content; zero-membership state shows no fabricated product; logout POST uses CSRF and clears session UI; 390px layout has no overflow. Preserve existing development-mode tests.

```ts
await page.route('**/auth/config', r => r.fulfill({json:{browser_login:true,development:false}}));
await page.route('**/v1/me', r => r.fulfill({status:401,json:{error:'unauthenticated'}}));
await page.goto('/');
await expect(page.getByRole('link',{name:'Sign in with your organization'})).toBeVisible();
await expect(page.getByLabel('Access token')).toHaveCount(0);
```

- [ ] Run `npm test -- session.spec.ts`; expected missing sign-in view.
- [ ] Implement Session with abortable loading, allowed product selector and no cached context. Keep current workbench views and design. CSRF stays in JS memory; no credential storage. On 401 clear session data; logout clears context even if server reports expiry but does not hide non-authentication failures. Vite proxies `/auth` as well as `/v1`.
- [ ] Run build and all browser tests; expected PASS; commit Task 4.

### Task 5: Local-provider live acceptance and deployment documentation

**Files:** Create `scripts/portal-identity-smoke.py`, `docs/reviews/2026-10-10-hosted-identity-acceptance.md`; modify backend/console docs, Compose variables and completion matrix.

**Interfaces:** Consumes complete hosted flow. Synthetic provider emits standard discovery/JWKS/auth/token responses using existing test RSA fixture; validates PKCE and nonce and exposes only loopback. Smoke test accepts only disposable wk_test database and uses unguessable synthetic credentials kept in a private temporary directory.

- [ ] Run local smoke against real PostgreSQL/server/provider: anonymous -> login -> callback -> `/me` -> permitted product API -> CSRF-protected logout -> denied session replay. Include wrong-browser callback, token/nonce rejection, cross-origin mutation, MCP cookie rejection and membership revoke; no real identity or raw conversation access. Expected PASS for each named scenario.
- [ ] Run `scripts/cargo fmt --check`, `scripts/cargo clippy -p wk-server -p wk-knowledge --all-targets -- -D warnings`, appropriate server/knowledge database tests, web build/browser tests and `git diff --check`. Expected all green; unavailable external production-provider login is recorded unverified.
- [ ] Document required issuer/client/callback configuration, no development provider in production, migration/app-role permissions, cookie proxy assumptions, logout semantics and exact local acceptance. Perform one independent review of this increment, fix important findings with RED→GREEN regressions, commit; continue later portal increments from their own implementation plans.
