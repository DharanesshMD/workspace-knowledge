# Public Alpha Preparation Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans inline. Reuse the attached candidate worktree, preserve unrelated work, and request one fresh final reviewer.

**Goal:** Prepare a reviewable source-only `v0.1.0-alpha.1` publication candidate with explicit privacy, dependency, contribution and support boundaries.

**Architecture:** Keep the existing server/collector trust boundaries and manual reviewed publication. Prepare repository hygiene and a maintainer decision packet; do not change runtime cryptography, hide dependency audit failures, merge, change visibility or publish before final approval.

**Tech Stack:** Existing Rust/PostgreSQL/React application; Markdown/YAML; Python standard library audit tooling; GitHub native reporting.

**Spec:** The user's approved six-step public-alpha sequence in this conversation; the owner has confirmed monitoring GitHub private vulnerability reports.

## Global Constraints

- Public alpha target is source only, Linux Docker server plus macOS developers. Existing unsigned binaries remain private candidates.
- Preserve production OIDC/TLS, Apple signing, real-server/team pilot and broader model/capture qualification as later gates. Automatic publication remains disabled.
- Retain unsuppressed RUSTSEC-2023-0071 and the independently reviewed public-key-only OIDC trace. A final maintainer alpha disposition is required; no security exception is presumed.
- Audit only repository source/history and owned synthetic fixtures. Never capture live conversations, inspect global account credentials, rewrite history or alter unrelated services.
- The repository owner monitors GitHub private reports after launch; do not invent an SLA or claim that public-only reporting is already enabled.
- Existing authorization covers candidate commits/pushes and draft PR updates. Final merge/public visibility/tag publication remain pending approval of concrete artifacts.

## Review Focus

- Historical files/media may retain private information even when a token scan passes.
- README/status prose must distinguish an actual local host test from cloud, native tool availability and production claims.
- Source-only alpha packaging must not accidentally publish unsigned binaries, captured histories, test credential state or claim a clean audit.
- Supported installation instructions must work from a fresh source tree without private tooling symlinks or developer-specific paths.
- Dependency/asset license declarations and a monitored reporting owner are required; an SBOM alone does not establish final distribution compliance.

### Task 1: Repository publication inventory and decision packet

**Files:** Create `docs/reviews/2026-10-12-public-alpha.md`; update `docs/security/rsa-runtime-disposition.md` with an explicit pending maintainer decision. Private intermediate inventory lives in this plan's ignored workspace.
**Interfaces:** Consume exact source/history/object inventory and previous bounded acceptance. Produce sanitized findings and explicit unresolved publication gates.

- [x] Inventory all reachable historical objects, archive/database/log/media types, path/reference metadata and current private-state exclusions with Python/Git. Expected: metadata-only inventory; inspect every relevant unique artifact without exposing real secrets.
- [x] Review source/dependency/asset rights and stored synthetic test keys; distinguish public fixtures from live credentials. Expected: retained notices or concrete missing obligations, never a directory-wide scanner exception.
- [x] Prepare the owner alpha security decision with the existing independent trace and unchanged red dependency gate. Expected: `pending approval`, not fabricated security acceptance.
- [x] Record exact findings, known privacy/capture/performance limits and checks completed. Do not certify unreviewed history or inaccessible assets.

### Task 2: Public-alpha documentation and community files

**Files:** Update `README.md`, `CONTRIBUTING.md`, `SECURITY.md`, `docs/operations/v0.1-release.md`; create `CHANGELOG.md`, `.github/ISSUE_TEMPLATE/config.yml`, `.github/PULL_REQUEST_TEMPLATE.md` and `docs/operations/public-alpha.md`; add a private vulnerability form only if it improves the existing native channel.
**Interfaces:** Consume Task1's reviewed scope and pending decision. Produce honest supported workflows, exact report ownership and a source-only launch runbook.

- [x] Correct stale README/CONTRIBUTING claims, describe source-only alpha and failed model-sensitive qualification without weakening publication controls.
- [x] Identify the owner as report monitor, document GitHub private reporting after visibility change and keep security findings out of public issue forms. Add contributor PR checks and safe feature/bug issue routing using native GitHub files.
- [x] Add concrete unreleased alpha notes and ordered integration/publication steps, including approval, native private reporting enablement and verification. No tag/release is created by writing notes.
- [x] Check local Markdown targets and YAML syntax; use existing tooling/stdlib rather than introducing a linter dependency. Expected: no missing internal file targets; no unsupported security-channel or release claim.

### Task 3: Clean-source acceptance, final review and handoff

**Files:** final candidate documents plus existing tests; candidate PR body. No runtime feature planned.
**Interfaces:** Consume prepared source-only candidate. Produce verified setup evidence, one fresh review, committed/pushed reviewable result and an approval-ready decision packet.

- [x] Export only tracked candidate source to a fresh owned directory, excluding private Cargo/tool symlinks and generated state. Build/demo with the real pinned Cargo; run appropriate existing Python checks. Expected: demo passes using fresh source and synthetic temporary state, no global install/profile mutation.
- [ ] Re-run redacted source/history scan and validate docs/YAML after final edits. No need to repeat unchanged PostgreSQL/model/load suites; retain exact prior evidence and failed gates.
- [ ] Request one fresh whole-change review covering Task1 inventory and docs/support/approval claims. Fix reproduced Important findings with meaningful regression or document correction; defer minor findings explicitly.
- [ ] Commit/push candidate, update the existing draft PR and assess exact-head checks. Prepare final approval request only after the candidate/remaining risk decision is concrete. Repository remains private unless the user approves that step.
