# OIDC RSA remediation acceptance — 2026-10-13

Scope: source-alpha preparation for Linux Docker server + macOS developers. Baseline `435dbc3`. This record is not production-provider, universal-capture or signed-distribution certification.

## Changed dependencies and trust boundary

Server/collector share the exact Convex OIDC fork revision `15a9c551eafcf3b60d307feb94332352ac56c099`. AWS-LC replaces RustCrypto RSA/ECDSA signature operations; application protocol verification remains in the established library. No timestamp-relaxation feature or custom JWT verifier was added. Fork provenance/MIT obligations and update review are described in the [assessment](../security/rsa-runtime-disposition.md).

The fork's workload-identity support makes an authorization endpoint optional and defaults omitted response/subject/signing-algorithm lists to empty. Both interactive login paths now reject those states. Server and native regression fixtures first demonstrated accepted incomplete discovery, then passed after explicit rejection. Existing HTTPS/literal-loopback, response size, no-redirect, issuer, audience, nonce, expiry, PKCE, browser binding, replay and access-token-hash checks remain intact.

The general SQLx facade retained unused MySQL/RSA packages in the full lockfile. A 13-line PostgreSQL-only crate re-exports exact official SQLx core/driver0.8.6 APIs and original Tokio/Rustls-ring-WebPKI mapping. It adds no query or crypto behavior. Full PostgreSQL suites exercise the unchanged row-level roles, transactions and revocation controls. Separate local SQLite remains unchanged.

## Local verification

- Existing all-target runtime-RSA gate: **RED before → GREEN after**. Full lockfile contains no rsa, sqlx-mysql or sqlx-sqlite package.
- Unsuppressed Cargo audit: **zero vulnerabilities**, one informational **RUSTSEC-2025-0134 rustls-pemfile unmaintained** warning. No ignore/allowlist or audit-policy relaxation. That parser belongs to the fork's private-key helper, which production consumers do not use; replacement remains a maintenance follow-up.
- npm audit: zero vulnerabilities, including development packages at scan time.
- Pinned Cargo fmt/clippy/workspace build passed. **282 workspace Rust/PostgreSQL tests**, zero failed/ignored, plus **three privacy-example and one pilot-example tests** passed (286 total). An owned PostgreSQL17 wk_test fixture only; no production database used.
- **Eight Python tests** passed, including real merge-only credential detection with checksum-verified Gitleaks8.30.1. CLI consent/scope/replay/pause demo and real HTTP/MCP integration passed.
- TypeScript/Vite build and **34 Playwright tests** passed with synthetic UI content.
- Public and confidential local portal acceptance: **14 checks each**, covering correct token hash, forged signature, wrong issuer/audience/nonce, expiry, wrong hash, browser binding/replay, scope, CSRF/origin, revocation and logout. A new positive callback test initially expected HTTP200 incorrectly; corrected to the existing303 redirect contract and reran successfully. No product behavior was relaxed to pass it.
- The upstream fork's **69 library tests** passed from an owned copied source with its reqwest-blocking test feature. The first no-default-feature attempt did not compile test helpers referencing reqwest; enabling the test-only feature resolved that harness requirement. The application continues to use custom bounded HTTP with library HTTP features disabled. Cached dependency source was not edited.

## Final review and exact-head checks

The final independent review, corrected findings and exact-head hosted CI are recorded in the candidate PR after verification. A local audit/build is not evidence that every CI platform passed. Source-only candidate builds remain disabled in GitHub Actions and protected by private-repository job conditions. No binary, merge, public visibility or tag/release was published by this remediation.

The strict actual-model privacy corpus remains failed for its contact-sensitive case, although application screening held the observed output. Automatic publication remains disabled. Production OIDC/TLS, real-server/team/long-duration capacity, expanded capture/privacy qualification and Apple signing/notarization remain later gates.
