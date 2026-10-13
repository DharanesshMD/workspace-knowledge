# RSA dependency remediation

## Current candidate — 2026-10-13

The previous RustCrypto RSA gate is remediated in the candidate source. Server and collector share the OpenID Connect API from [Convex's fork](https://github.com/get-convex/openidconnect-rs/commit/15a9c551eafcf3b60d307feb94332352ac56c099), pinned to immutable revision `15a9c551eafcf3b60d307feb94332352ac56c099`. It replaces RustCrypto signature operations with AWS-LC, already used by our access-token verifier. This is an external fork, not a patched release of the RustCrypto `rsa` crate or a custom application JWT verifier.

The generic SQLx facade also resolved the unused MySQL driver and its RSA dependency into Cargo.lock even with PostgreSQL-only runtime features. `wk-postgres` now only re-exports exact official SQLx core/PostgreSQL 0.8.6 types and functions, preserving the Tokio runtime and Rustls/ring WebPKI TLS mapping. It adds no query, pooling, transaction or cryptographic implementation. MySQL/SQLite facade resolution is removed; the application's separate local SQLite storage is unchanged.

The all-target normal dependency graph **and full lockfile contain no `rsa` package**. Unsuppressed `cargo audit` currently reports **zero vulnerabilities**, with one informational maintenance warning: **RUSTSEC-2025-0134, rustls-pemfile unmaintained**. No advisory ignore or scanner exception was added. That parser is an upstream dependency of the fork's private-signing-key helper; our production OIDC consumers verify public JWKs and do not construct its private signing keys. Track upstream replacement; zero reported vulnerabilities is not security certification.

The fork permits incomplete discovery for workload identities. Both interactive login paths explicitly reject missing authorization endpoints and empty required response/subject/signing-algorithm lists. No relaxed timestamp feature is enabled. Established issuer, audience, expiry, nonce, PKCE, browser binding, access-token-hash and bounded/no-redirect HTTP checks remain in place. New regressions cover incomplete discovery, forged signatures, HMAC algorithm confusion and wrong claims; local public/confidential portal flows exercise positive and negative token-hash cases. See the [remediation acceptance](../reviews/2026-10-13-oidc-rsa-remediation.md) for actual results and exact-head CI.

## Maintenance and provenance

The fork declares MIT licensing; retain its dependency notice/license text in any later binary distribution. This pin is also used by [Convex's backend manifest](https://github.com/get-convex/convex-backend/blob/main/Cargo.toml). Source adoption does not constitute independent certification or an upstream promise of support. Review future fork revisions before updating; reassess discovery and algorithm semantics, test both login paths and rerun the full audit. The exact SQLx internal runtime/TLS feature mappings likewise require review on upgrades.

No owner acceptance of RUSTSEC-2023-0071 is required for this candidate because the affected package was removed. Public visibility, metadata disclosure and source-only release approval remain separate actions. Production-provider/TLS and signed distribution acceptance remain pending.

---

The following is preserved evidence for the old crates.io-based candidate. Its red-audit/disposition statements are **superseded** by the current remediation above; they are not current release status.

# Historical assessment — 2026-10-12

Assessment date: 2026-10-12. Scope: the pinned v0.1 candidate application paths, `openidconnect 4.0.1` and `rsa 0.9.10`; not a general cryptographic or production audit.

## Dependency and advisory

Both `wk-server` and `wk-agent` normally link `rsa` through `openidconnect`. This is **not development-only**. The [primary RUSTSEC-2023-0071 advisory](https://rustsec.org/advisories/RUSTSEC-2023-0071.html), rechecked on this date, reports private-key recovery through timing and no patched version, including the current stable/prerelease lines. An upstream prerelease or disabling optional OIDC features is not a demonstrated remediation.

`cargo audit` stays unsuppressed and the dependency CI gate remains red. No exception or new custom OIDC verifier was introduced. A static applicability assessment is not a dependency patch or permission to disregard future changes.

## Inspected call paths

- Server `BrowserAuth::complete` in `crates/server/src/browser_auth.rs`: authorization-code exchange with PKCE, `client.id_token_verifier()`, `id.claims(verifier, nonce)`, optional access-token-hash check. The OIDC client secret is a symmetric protocol credential, not an RSA private key.
- Collector `Login::complete` in `crates/agent/src/native_login.rs`: public-client authorization-code exchange with PKCE, the same established ID-token verifier and nonce/access-token checks. Its separate device signing key is Ed25519.
- Pinned OIDC `verification/mod.rs`: selects a provider verification key and calls `jwt.payload`; `CoreJsonWebKey::verify_signature` in `core/jwk/mod.rs` dispatches RSA algorithms to `core/crypto.rs::verify_rsa_signature`.
- That function extracts only modulus/exponent, constructs `rsa::RsaPublicKey`, and invokes public signature verification. It does not construct or operate on an RSA private key. The pinned library also defines `CoreRsaPrivateSigningKey`; no application production entry point using it, RSA signing, RSA decryption or a private RSA exponent was found in the inspected consumers.
- Ordinary server access-token verification uses `jsonwebtoken 10.4` with the AWS-LC backend in `crates/server/src/auth.rs`. Synthetic provider/load fixtures sign test RSA keys using OpenSSL; those are test facilities, not production OIDC private-key operations through this crate.

The inspected production OIDC verification paths therefore do not expose the private-key operation described by this advisory. This conclusion is limited to those paths and this pinned dependency graph. It does not establish absence of other vulnerabilities, resource exhaustion, unsafe future code or deployment-specific risk. Reassess before adding RSA signing/decryption, a provider role, dependency changes or other private-key functionality.

## Disposition

Retain the established OIDC implementation, track upstream remediation, and keep the audit result visible. A fresh independent continuation reviewer corroborated the trace through public exponentiation and found no affected private-RSA operation reachable in the inspected consumers. Its scope and remaining findings are recorded in the acceptance report. Production provider/TLS acceptance and release authorization remain separate gates. Do not describe this assessment as third-party security clearance or a clean dependency audit.
