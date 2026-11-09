# Release qualification implementation plan

**Goal:** qualify the local MCP candidate against current dependency, secret and remote CI checks, fixing reproduced failures while preserving the evaluation release boundary.

**Scope:** user's instruction to proceed with the next qualification step after repeatable onboarding. Existing [production gates](../../operations/production-readiness.md) remain authoritative. No visibility change, release publication, billing changes, new infrastructure or capture-pin bypass.

**Approach:** reuse the existing audit tools, CI workflows and acceptance scripts. Keep dependency fixes narrow; preserve the local deployment and user credentials. Use the current branch and commit tested fixes before dispatching its existing remote workflows.

- [x] Read current remote repository/workflow state and compare it with the historical readiness ledger.
- [x] Reproduce Tokio's unsoundness warning with `cargo audit --deny unsound`; update only the patched Tokio lockfile dependency and verify the warning disappears.
- [x] Run checksum-verified Gitleaks on branch/remote/tag history and the merge-only regression; run all Python checks without the previous scanner skip.
- [x] Run Rust/database checks and release build after the dependency change, refresh local installation as appropriate, and preserve working MCP connections.
- [x] Push the private candidate branch and dispatch Foundation and Dependency workflows on its exact commit. Inspect failures, fix code failures, rerun when justified, and record external blockers precisely.
- [x] Record verification and unresolved release gates in a dated report and readiness update; commit the reviewable result.

Review focus: no suppression of an unsoundness finding; no accidental unrelated dependency upgrades; no secret values in scanner output; no confusing capture drift with MCP support; no claiming a local or historical run certifies the final release commit.

Remote findings at `1f4593b`: the Linux stale-folder test reproduced inode reuse bypassing the inode-only approval fingerprint, and the PostgreSQL job exhausted runner disk with debug/incremental builds. Use generation-aware export handles on Linux and creation time on other Unix systems, fail closed when required metadata is unsupported, require fresh approval for legacy bindings, protect ordinary-edit stability and rapid root replacement with regressions, and bound PostgreSQL build output. Windows also exposed fixture CRLF, an illegal test folder name, and parent traversal hidden in a verbatim path component; fix these without skipping the tests. Rerun the candidate's complete workflows after these fixes.

Outcome: all seven Foundation jobs and both Dependency jobs passed at `49adaaf`. The final test-only change preserves literal traversal input because `PathBuf::join` normalizes it for Windows verbatim prefixes. Final local application verification passed 803 Rust tests, 58 Python checks without skips, 77 browser tests and all 12 onboarding lifecycle checks. See the [qualification report](../../reviews/2026-11-08-release-qualification.md) for the exact snapshots, CI links, local deployment and remaining production gates.
