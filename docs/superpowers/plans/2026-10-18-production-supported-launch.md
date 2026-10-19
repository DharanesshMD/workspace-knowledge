# Production-Supported Launch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prepare the approved Linux Docker + macOS support scope for a production-supported public launch without changing GitHub repository visibility.

**Architecture:** Split repo-contained remediation from acceptance requiring external owner resources. Keep capture and publication boundaries unchanged; use exact-version evidence to qualify support rather than expanding claims. A release readiness ledger will record each gate, owner, evidence and exact revision; unresolved external gates stop any production-ready claim.

**Tech Stack:** Rust workspace and PostgreSQL, React/Vite/Playwright console, Python acceptance harnesses, Docker Compose, GitHub Actions, macOS Keychain/launchd distribution.

**Spec:** [Production-Supported Launch Design](../specs/2026-10-18-production-supported-launch.md)

## Global Constraints

- Supported scope is Linux Docker server + macOS developer machines; qualify exact architectures, client versions, and surfaces before advertising them.
- Automatic publication remains disabled; ordinary retrieval remains advisory.
- Do not change repository visibility; the owner changes it manually.
- Never use real private conversations or production data as fixtures.
- Do not mark a gate complete without evidence tied to exact commit, environment, OS, client version, and workflow.
- Production OIDC provider, signing credentials, pilot resources/participants, independent security review, and GitHub billing require owner/external input; do not invent or provision them.

## Review Focus

- Unsupported/mixed client sessions must remain partial or held, never be inferred complete; add assertions to the owned adapter qualification tests.
- Model-sensitive or injection-framed content must not publish on ambiguous model output; extend `crates/summarize/tests/privacy.rs` and the privacy acceptance harness.
- Expired, revoked, paused, or offline devices must not appear to have controls applied sooner than observed; extend device/portal acceptance coverage where review finds gaps.
- Clean-machine install/upgrade/uninstall/rollback must not overwrite unrelated files or leave credentials behind; extend source-install tests and signed artifact acceptance.
- OIDC callback, issuer, audience, cookie, and TLS proxy misconfiguration must fail closed with actionable operator errors; extend the provider integration tests for the selected issuer.

---

## Phase A — Repository truth and usability (repo-contained)

### Task 1: Reconcile release-state documentation

**Files:**
- Create: `docs/operations/production-readiness.md`
- Modify: `README.md`
- Modify: `CHANGELOG.md`
- Modify: `SECURITY.md`
- Modify: `docs/operations/public-alpha.md`
- Modify: `docs/operations/v0.1-release.md`
- Modify: `docs/architecture/completion-matrix.md`
- Modify: `docs/operations/install.md`
- Test: documentation link/status checks and release-history scan

**Interfaces:**
- Consumes: published GitHub release/tag state and current acceptance reports.
- Produces: one consistent description of alpha.3, current support boundaries, production gates, disclosure channel, install workflow, and a gate ledger tied to evidence.

- [x] **Step 1: Add a failing documentation consistency test** that checks README, changelog, SECURITY, install and release checklist do not call the already-published alpha.3 “unpublished”, point evaluators at alpha.1, or claim production support before gates pass.
- [x] **Step 2: Run the focused test and confirm it fails** on the stale references currently observed.
- [x] **Step 3: Create the readiness ledger and update the named docs** with exact current release/tag status; keep alpha.3 historical, label production readiness as blocked, and avoid promising unqualified clients/platforms.
- [x] **Step 4: Run documentation/link checks and the publication-history scan**; confirm all references resolve and no newly introduced private paths/secrets appear.

### Task 2: Validate clean install through first team workflow

**Files:**
- Review/modify as evidence requires: `README.md`, `docs/operations/install.md`, `docs/adapters/setup.md`, `deploy/scripts/install-source.sh`, `scripts/tests/test_source_install.py`
- Test: `scripts/demo.py`, owned source-install harness, existing portal Playwright suite

**Interfaces:**
- Consumes: corrected support boundary from Task 1.
- Produces: repeatable, documented path for clean install → demo → provisioned login → first reviewed summary/retrieval, with unsupported stages called out.

- [x] **Step 1: Add a clean-prefix acceptance case** asserting install creates only the documented binaries, prints actionable next steps, and does not enable capture, startup, or publication implicitly.
- [x] **Step 2: Run the new case red** against the current install flow.
- [x] **Step 3: Make the smallest install/usability fixes** needed for the failing assertions, preserving explicit consent and existing service behavior.
- [x] **Step 4: Run fresh-prefix install, synthetic demo, portal browser suite at desktop and phone widths, uninstall, and exact rollback verification.** Record what still requires an administrator/provider.

## Phase B — Product acceptance (repo-contained plus named external gates)

### Task 3: Qualify identity and HTTPS deployment

**Files:**
- Review/modify as evidence requires: `crates/server/src/` OIDC/session modules, `scripts/integration.py`, `scripts/portal-device-smoke.py`, `.github/workflows/ci.yml`, `docs/architecture/remote-mcp.md`, `docs/operations/install.md`
- Test: synthetic provider regressions and selected-provider staging smoke

**Interfaces:**
- Consumes: owner-selected issuer, portal/collector audiences, callback URLs, and disposable staging environment.
- Produces: fail-closed provider configuration and exact evidence for PKCE/state/nonce, issuer/audience separation, key rotation, expiry, logout, TLS proxy behavior, and documented recovery.

- [x] **Step 1: Owner selects provider and supplies a disposable staging test path.** Owner declined a hosted provider; agreed on a disposable local Keycloak over private-CA HTTPS instead. The synthetic provider is still not production evidence.
- [x] **Step 2: Add failing provider-specific tests** for the documented callback, audience, expiry/rotation and misconfiguration cases. (`scripts/keycloak-acceptance.py`; CA-bundle unit test.)
- [x] **Step 3: Implement only confirmed defects**, using established OIDC/session code and no custom token-verification protocol. (`WK_OIDC_CA_FILE` for private-CA providers.)
- [ ] **Step 4: Run the full synthetic matrix and selected-provider HTTPS staging smoke**; record issuer/version, configuration, commit and sanitized results. (Local Keycloak 42/42 recorded in the readiness ledger; repeat at the final commit and against any hosted provider the owner later selects.)

### Task 4: Pass privacy and supported-client qualification

**Files:**
- Review/modify as evidence requires: `crates/summarize/tests/privacy.rs`, `crates/summarize/examples/privacy_acceptance.rs`, `scripts/history-eval.py`, `docs/architecture/completion-matrix.md`, `docs/adapters/setup.md`, relevant adapter harnesses under `scripts/`
- Test: expanded synthetic sensitive/injection corpus and named client/version acceptance

**Interfaces:**
- Consumes: owner-approved client/version matrix and privacy corpus expectations; no real personal data.
- Produces: deterministic pass/fail evidence and per-surface support labels; no automatic-publication enablement.

- [ ] **Step 1: Expand synthetic corpus coverage** for contact/credential/personal-data variants, prompt injection, malformed/model-unavailable output, multilingual/Unicode inputs, truncation and ambiguous partial sessions. (Contact/email/phone/address, Unicode phone, and oversized input were added; malformed/model-unavailable, truncation and ambiguous-session coverage still need qualification.)
- [x] **Step 2: Confirm strict harness fails on the known contact-sensitive miss and on provider errors misclassified as holds.** (The contact miss was reproduced; the harness already asserts provider outage and malformed output are not privacy holds, and the contact detector's identifier false positives were found and fixed 2026-10-19.)
- [x] **Step 3: Fix only reproduced screening/acceptance defects**; keep uncertain drafts private/held and record false-positive/false-negative limitations.
- [ ] **Step 4: Run named client/version tests on owned isolated profiles** and update exact support claims; unsupported host surfaces remain explicitly unsupported.

### Task 5: Qualify macOS distribution and rollback

**Files:**
- Review/modify as evidence requires: `.github/workflows/release.yml`, `scripts/release-manifest.py`, `docs/operations/offline-bundle.md`, `docs/operations/install.md`, release tests
- Test: clean macOS arm64 install/upgrade/uninstall/rollback and signature verification

**Interfaces:**
- Consumes: owner-managed Apple Developer signing/notarization credentials and approved release identity.
- Produces: signed/notarized artifacts, checksums, dependency/license inventory, and verified install/rollback evidence.

- [ ] **Step 1: Owner provides an approved isolated signing path**; never place certificate/private key material in source, logs, command arguments, or artifacts.
- [ ] **Step 2: Add release-manifest tests** for expected binaries/assets, signatures, license/notice material, checksum verification and rejection of unsigned candidates.
- [ ] **Step 3: Implement the minimum protected signing/notarization workflow** only after credential handling is reviewed.
- [ ] **Step 4: Verify a clean-machine install, first run, update, uninstall, exact rollback, Gatekeeper/signature status and retained dependency notices.**

## Phase C — Operations, security, and launch evidence

### Task 6: Run target-team capacity and recovery acceptance

**Files:**
- Review/modify as evidence requires: `scripts/hosted-pilot.py`, `scripts/hosted-restore-drill.py`, `scripts/restore-drill.py`, `docs/operations/hosted-portal.md`, `docs/operations/backup.md`, `docs/operations/restore.md`, `docs/operations/v0.1-release.md`
- Test: disposable target-like Linux Docker + PostgreSQL environment with synthetic workload, then owner-approved pilot

**Interfaces:**
- Consumes: representative, consented pilot workload and target infrastructure supplied/approved by owner.
- Produces: measured capacity envelope, latency/error/recovery results, restore/deletion evidence, and operator runbook with explicit RPO/RTO assumptions.

- [ ] **Step 1: Define workload, duration, success thresholds, retention horizon, and budget with the owner before provisioning anything.**
- [ ] **Step 2: Extend the owned pilot harness** to report p50/p95, errors, database CPU/IO, storage/WAL growth, recovery time and deletion/restore outcomes with content-free output.
- [ ] **Step 3: Run synthetic target-like load and recovery tests**; fix regressions and repeat at the exact final revision. (Local synthetic pilot and 14-check restore drill passed 2026-10-19; repeat at the final revision on target-like infrastructure.)
- [ ] **Step 4: Run the owner-approved real-team pilot** and document the measured supported envelope; do not extrapolate beyond observed workloads.

### Task 7: Close security/release controls and final verification

**Files:**
- Review/modify as evidence requires: `SECURITY.md`, `.github/workflows/ci.yml`, `.github/workflows/security.yml`, `.github/workflows/release.yml`, `docs/operations/public-alpha.md`, `docs/operations/v0.1-release.md`, `docs/reviews/`
- Test: exact-head Rust/Python/web/container checks, dependency audit, secret/history scan, GitHub Actions run/artifact/log inventory, independent security review

**Interfaces:**
- Consumes: billing-enabled exact-head CI runners, external independent reviewer, resolved documentation and acceptance results.
- Produces: go/no-go ledger tied to SHA; current reporting instructions and verified clean CI/security/release evidence.

- [ ] **Step 1: Resolve the GitHub billing/spending-limit block** through the account owner; do not attempt to evade runner restrictions.
- [ ] **Step 2: Run full local checks and exact-head GitHub CI**; dependency audit and history/secret scans must be green, with warnings explicitly dispositioned.
- [x] **Step 3: Refresh the full retained Actions log/artifact inventory and review historical disclosure findings**; do not delete remote evidence without explicit authorization.
- [ ] **Step 4: Obtain independent security review** and disposition all findings with evidence; document only the actual private-reporting channel available at launch time.
- [x] **Step 5: Run final go/no-go review against every criterion in the spec**; visibility remains private until the owner manually changes it. (No-go: production gates remain open.)

## Deferred pending explicit owner choices

- Production OIDC provider and its disposable staging configuration.
- Supported client/version matrix beyond the evidence already available.
- Apple signing account/identity and protected credential hosting.
- Pilot participant(s), target infrastructure, workload, success thresholds and budget.
- Independent security reviewer and review scope.
- GitHub billing repair and the final visibility-change timing.

## Plan self-review

- **Spec coverage:** Support boundary and consent constraints are covered in Tasks 1–5; OIDC/TLS in Task 3; privacy/client acceptance in Task 4; distribution in Task 5; capacity/restore in Task 6; independent security review, exact-head CI, documentation and visibility constraint in Tasks 1 and 7.
- **Step scan:** Each phase has an observable test or owner-provided external gate; no external environment or credential is assumed to exist.
- **Type consistency:** Phase outputs are documentation/evidence and do not add an unapproved protocol, service, or publication behavior.
- **Review focus:** Each listed failure class maps to a named test task.
- **Proportion:** This plan is a release-level orchestration plan; subsystem-specific implementation plans should be written after owner choices resolve provider, client matrix, signing, pilot and reviewer scope.
