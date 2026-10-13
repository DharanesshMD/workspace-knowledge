# Hosted identity: local acceptance

Date: 2026-10-10. Scope: increment 1 of the approved hosted portal roadmap. The user selected a local test provider; no external production-provider login is certified.

Implemented: configured OIDC authorization-code + PKCE sign-in, nonce/state and browser binding, opaque sessions, CSRF and exact-origin checks, fresh identity/membership lookup, scoped product selection, logout, and production setup-needed behavior without token-paste fallback. Browser cookies do not authenticate MCP. No raw conversation or provider token endpoint was introduced.

## Evidence

- Real Rust server + PostgreSQL + loopback synthetic OIDC provider: all nine named scenarios passed for both public and confidential clients in `scripts/portal-identity-smoke.py`: browser binding/replay, product listing, product API scope, MCP cookie rejection, CSRF/origin rejection, wrong nonce, wrong signature, membership revocation and logout. The synthetic provider checks client credentials, redirect URI and PKCE verifier, and signs ID tokens using a disposable fixture. No real identity/agent chat is accessed.
- Real in-app browser: sign-in reached trusted user alice with only beta visible after alpha was revoked; Sign out returned to the anonymous organization sign-in screen.
- Server regression suite: 17 passed, two PostgreSQL tests deferred to the database run. A failing regression exposed missing no-store/referrer headers on rejected requests; it passed after middleware covered both success and failure.
- PostgreSQL session regressions: two passed, including idle/absolute expiration, deletion, identity remapping, current memberships and a non-owner runtime role. The restricted-role test failed on session creation before fixing grants and replacing identity-row locking with an advisory lock. That role still cannot provision identities.
- Web build and ten Playwright regressions passed. Desktop and 390px screenshots were inspected. The desktop Sign out control initially extended below the viewport; its regression failed and then passed after the hosted shell accounted for the session header.
- Formatting, clippy with warnings denied, full relevant PostgreSQL regression suite and diff whitespace checks are recorded in the final verification below.

## Dependency security and limits

`openidconnect 4.0.1` brings `rsa 0.9.10` into the runtime graph. The previous audit ignore claimed rsa was an unused optional SQLx driver; that claim is no longer true and the ignore was removed. The unfiltered audit reports **RUSTSEC-2023-0071**. [RustSec](https://rustsec.org/advisories/RUSTSEC-2023-0071.html) describes timing leakage during private-key operations and has no patched version. This application only verifies provider public signatures; it neither holds nor signs/decrypts with an RSA private key in its authentication runtime. This reduces applicability of the described key-recovery attack, but does not make the dependency audit clean. A reviewed dependency replacement or upstream fix remains a release follow-up. Synthetic private-key token signing runs only in the disposable test provider through OpenSSL.

This increment does not implement owner-only hosted summaries, collector synchronization, a team feed, hosted device controls, admin roles or measured capacity. Existing unpublished cards remain visible to scoped reviewers. Pending logins live in one process; a restart requires restarting sign-in, and multi-replica login needs shared transactions or affinity. Portal logout does not log out the provider. No production-provider or native OS release certification is claimed. TLS, proxy logging/rate limits, secret provisioning and production restore/load acceptance remain operator rollout obligations.

## Final verification

Formatting and clippy (server/knowledge, all targets, warnings denied) passed. All 20 relevant PostgreSQL regressions passed: 16 access/lifecycle/RLS, two browser sessions, one safe HTTP error and one native handoff. Web production build and ten browser tests passed. Python smoke syntax and `git diff --check` passed. Unfiltered cargo-audit reports one advisory (RUSTSEC-2023-0071), with no ignored advisories. These are local test results, not a production load or provider certification.

Browser regression screenshots (synthetic alice/beta identity): [desktop](hosted-identity/desktop.png), [390px](hosted-identity/mobile.png).

Independent review completed. Important findings were fixed in one pass: empty client secrets now select public-client exchange (live smoke observed fail then pass); concurrent failed metadata refreshes share a 60-second cooldown (five requests observed five fetches before the fix and one afterwards); development console origins are restricted to literal loopback (regression observed fail then pass). Logout acceptance now replays the saved pre-logout cookie and receives 401 for both client modes. No deferred review findings remain.

Implementation decisions: retain metadata refresh in the existing authentication service with a bounded snapshot/cooldown, rather than add a worker; derive CSRF with domain separation from the opaque session secret; serialize per-identity session creation using an advisory lock to preserve restricted identity permissions. A mistaken lock choice could undermine the session cap, so the least-privilege database regression covers the path. Production provider and capacity claims remain withheld.
