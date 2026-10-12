# Runtime RSA advisory assessment

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

## Pending source-only alpha decision

**Status: pending repository-owner approval.** The proposed narrow disposition is to permit a source-only experimental alpha with this advisory disclosed, the established OIDC verifier retained and the dependency audit left visibly failing. It would not authorize production deployment, binary distribution, private-RSA operations, an audit exception or a claim of security clearance. Remediation remains tracked; dependency or cryptographic changes require reassessment.

Before launch, the owner must explicitly accept or reject this scoped disposition after reviewing the inspected call paths and independent review evidence. Approval to prepare documentation or monitor reports is not acceptance of the advisory. Until that decision, public visibility, merge and alpha tag/release publication remain pending. See the [concrete launch packet](../operations/public-alpha.md).
