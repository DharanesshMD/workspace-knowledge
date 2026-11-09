# Release qualification — 2026-11-08

Candidate: `49adaaf06d58ce4d7e425cd8b5093d396230b017` on the private `codex/local-ide-acceptance-20261107` branch. Application code is unchanged from `8ab74d8`; the last commit corrects only a Windows test fixture. This follows the [onboarding acceptance](2026-11-07-client-onboarding-acceptance.md). No main-branch merge, release publication or visibility change is part of this qualification.

**Outcome:** all seven Foundation jobs and both Dependency jobs passed at this exact candidate. Local installation and the 12-check onboarding lifecycle also passed. This qualifies the tested evaluation workflow; production release gates remain open.

## Fix and audit

The fresh audit reproduced [RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html) against Tokio 1.53.1: `cargo audit --deny unsound` exited 1. Updating the lockfile to the patched Tokio 1.53.3 made the same check exit 0. This is the only package-version change; Cargo also normalized eight existing Windows dependency edges to already-locked `windows-sys` versions. The dependency workflow now uses `cargo audit --deny unsound` so this warning class cannot silently recur.

The refreshed RustSec database was commit `309392b18f6b17022f82c701f300433126901068` (2026-11-06). Result: zero vulnerability findings, zero unsoundness warnings, and four unmaintained-package notices (`rustls-pemfile`, `zbus_names`, `zvariant`, `zvariant_derive`). The normal-runtime dependency graph across targets contains no `rsa` package. The production npm audit reports zero vulnerabilities. These are bounded automated checks, not security certification.

## Local evidence

The first local 801-test run qualified the Tokio patch. The final local run at `8ab74d8` passed 803 tests, including the added legacy-approval invalidation and ordinary-edit stability regressions. The additional Linux-only rapid-recreation test passed in the remote PostgreSQL job.

| Check | Result |
|---|---|
| Final Rust workspace, disposable PostgreSQL 17/pgvector, `--include-ignored` | 803 passed; no failures or ignored tests |
| Final literal-input consent fixture at `49adaaf` | All five consent tests passed locally; Windows CI passed the complete workspace suite |
| Strict all-target/workspace Clippy; formatting; whitespace | Passed |
| Python discovery with checksum-verified Gitleaks executable | 58 passed, no skips |
| Web production build and browser suite | Passed; 77 tests |
| Gitleaks 8.30.1 branch/remote/tag history | Final candidate: 294 commits scanned; no leaks |
| Normal-runtime RSA dependency gate | Passed across targets |
| Linux arm64 Docker runtime build | Passed |
| Read-only, non-root runtime/assets and missing-config startup refusal | Passed |
| Deployed HTTPS two-user sharing/retrieval verifier | Passed; its synthetic card withdrawn, HTTP 200 |
| Actual six-command release source install in normal user prefix | Passed after the Tokio update |
| Fresh HOME/prefix full onboarding lifecycle on patched native tools and server | 12 checks passed, including outage, expiry, revocation, replacement and cleanup |
| Focused independent dependency/CI review | No actionable findings |
| Private source archive and SPDX dependency inventory | Generated for `49adaaf`; both SHA256SUMS entries verified |

The Gitleaks archive checksum was verified against the upstream release checksum list before execution. A separate scan of the tracked source tree, staged without ignored/private state, reported four existing synthetic test fixtures: two masking canaries in `crates/adapters/tests/{cursor_history.rs,support/cursor_fixture.rs}` and the two `crates/server/tests/fixtures/synthetic-rsa-{a,b}.json` signing keys consumed by the OIDC tests. These are test data, not live credentials. No broad scanner exception was added. Local-only stash/internal refs are outside the branch/remote/tag history scan; GitHub's clean-checkout history job is the authoritative scan of remotely available refs.

The local app was updated first to the Tokio-patched image and then to `wk-server:release-qualified-20261108`, also tagged `wk-server:local`, image ID `sha256:8d73bab16a1a38b6e2fd9b1fa981be9f89dabd011d132a4f750b26f54317f1d3`, built from the application code at `8ab74d8`. The final test-only commit changes no application code. The previous `wk-server:local-ide-20261107-final` and intermediate `wk-server:release-check-20261108` images remain available for rollback. No database volumes, identity-provider configuration or user token files were replaced. The six source commands were rebuilt and installed after the consent fix. Final fresh HOME acceptance passed all 12 checks with receipt `.tooling/onboarding-acceptance/wk-clean-user-w4e0h2uw/results.json`; its disposable developer was removed and synthetic card withdrawn. Both normal local Codex/Antigravity checks then discovered all 12 tools, and the app was healthy. The separate disposable PostgreSQL test container was stopped.

The private artifact directory is `.tooling/release-check-20261108/candidate-49adaaf/`: an unsigned source archive, `dependencies.spdx.json` and `SHA256SUMS`, generated with the existing manifest tool. Its 594 archive members contain the project license and exclude Git metadata, private tooling state, build output and node_modules. The dependency inventory describes 314 packages selected for the current macOS host; it is not a full cross-platform license audit, build attestation or signature. Nothing was uploaded to a release.

## Remote evidence

The repository remains private, with published source prerelease `v0.1.0-alpha.3` and no retained Actions artifacts at inspection. Historical Foundation jobs at `572ec01` failed before any step because of account billing. The weekly 2026-11-02 client-drift job did execute, and the new qualification jobs also started; the historical billing result must not be treated as a current blanket runner block.

- Initial [Foundation checks](https://github.com/DharanesshMD/workspace-knowledge/actions/runs/42407135809) at `1f4593b`: macOS, console, OpenBao and amd64 image passed; Linux, Windows and PostgreSQL failed as described below.
- Initial [Dependency audit](https://github.com/DharanesshMD/workspace-knowledge/actions/runs/42407138087): passed, including runtime RSA exclusion, the unsoundness-denying audit, production npm audit, full remote history scan and merge-only-credential regression.
- Updated [Foundation checks](https://github.com/DharanesshMD/workspace-knowledge/actions/runs/42510058771) at `8ab74d8`: all six non-Windows jobs passed. Windows passed the CRLF and markup fixtures but still failed the traversal test because its input had already been normalized by `PathBuf::join`.
- Updated [Dependency audit](https://github.com/DharanesshMD/workspace-knowledge/actions/runs/42510061238): passed at the same commit, including both history and dependency jobs.
- Final [Foundation checks](https://github.com/DharanesshMD/workspace-knowledge/actions/runs/42511024694): passed at `49adaaf`; Linux, macOS, Windows, PostgreSQL, console, OpenBao and amd64 image jobs all succeeded.
- Final [Dependency audit](https://github.com/DharanesshMD/workspace-knowledge/actions/runs/42511027161): passed at the same commit, including both history and dependency jobs.

### Reproduced CI failures and fixes

| Failure | Fix |
|---|---|
| Linux recreated a directory with a recycled inode, leaving its inode-only approval valid | Use the [Linux opaque filesystem handle](https://man7.org/linux/man-pages/man2/open_by_handle_at.2.html) for object identity, with bounded native buffers and no unsafe fallback. Creation time alone was rejected in review because Linux [coarse timestamps can collide](https://docs.kernel.org/filesystems/multigrain-ts.html). Other Unix identities now include creation time. Add a Linux rapid-recreation regression and invalidate old Unix inode-only approvals. |
| PostgreSQL job exhausted runner disk while building with full debug/incremental output | Apply disabled debug information/incremental compilation and two build jobs, retaining the entire database suite. |
| Windows fixture text checked out with CRLF appeared stale | Normalize checkout line endings before the strict content comparison. |
| Windows could not create the UI-escaping test's `<`/`>`/quote directory name | Use a legal Windows name containing ampersand/apostrophe, retain escaping assertions, and retain hostile markup coverage in replacement text. |
| Windows traversal test used `join` with a verbatim prefix, which [normalizes away `..`](https://doc.rust-lang.org/std/path/struct.PathBuf.html#method.push) before validation | Preserve literal input through `OsString`/`PathBuf::from`. The production guard also checks both separator forms because forward slashes in literal verbatim paths can remain inside Normal components. |

The Linux handle buffer and Windows validation fixes received focused independent review with no remaining actionable findings. Export handles must be supported and permitted by the filesystem/kernel policy; otherwise local folder approval fails closed. Existing Unix local folder approvals require explicit reapproval and collector reenrollment. Personal-token proxy onboarding is unaffected.

The initial macOS runner's Python suite ran 58 tests with five explicit skips: three cases required an installed Codex CLI, one required Docker Compose, and one required the Gitleaks executable supplied only to the separate history job. The local 58-test run had no skips and covers those cases. Native Python checks are not run by the Windows workflow; its passing native build/test job does not qualify the POSIX onboarding helper on Windows.

The capture-drift check returned Cursor 3.29.9 versus qualified 3.23.23 and Claude Code 2.1.319 versus qualified 2.1.293. It fails as designed. No tested-version pin was advanced without capture qualification. This does not invalidate the separately tested Codex/Antigravity MCP workflow. Automatic capture remains outside the evaluation scope.

## Release boundary

The successful remote runs qualify candidate `49adaaf`; this report and the ledger are recorded in a subsequent documentation-only commit. A future release revision still needs its own complete checks. The private release-candidate artifact workflow was not dispatched, and no release binary or signed artifact was published. This Mac has zero valid code-signing identities. Clean physical-machine installation, signed/notarized distribution, a public identity provider, independent security review and a consented real-team target-server pilot remain open. The local source installation and fresh HOME acceptance do not establish that every user's system will behave identically. See the [production readiness ledger](../operations/production-readiness.md).
