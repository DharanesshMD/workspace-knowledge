# Release qualification implementation plan

**Goal:** qualify the local MCP candidate against current dependency, secret and remote CI checks, fixing reproduced failures while preserving the evaluation release boundary.

**Scope:** user's instruction to proceed with the next qualification step after repeatable onboarding. Existing [production gates](../../operations/production-readiness.md) remain authoritative. No visibility change, release publication, billing changes, new infrastructure or capture-pin bypass.

**Approach:** reuse the existing audit tools, CI workflows and acceptance scripts. Keep dependency fixes narrow; preserve the local deployment and user credentials. Use the current branch and commit tested fixes before dispatching its existing remote workflows.

- [x] Read current remote repository/workflow state and compare it with the historical readiness ledger.
- [x] Reproduce Tokio's unsoundness warning with `cargo audit --deny unsound`; update only the patched Tokio lockfile dependency and verify the warning disappears.
- [x] Run checksum-verified Gitleaks on branch/remote/tag history and the merge-only regression; run all Python checks without the previous scanner skip.
- [ ] Run Rust/database checks and release build after the dependency change, refresh local installation as appropriate, and preserve working MCP connections.
- [ ] Push the private candidate branch and dispatch Foundation and Dependency workflows on its exact commit. Inspect failures, fix code failures, rerun when justified, and record external blockers precisely.
- [ ] Record verification and unresolved release gates in a dated report and readiness update; commit the reviewable result.

Review focus: no suppression of an unsoundness finding; no accidental unrelated dependency upgrades; no secret values in scanner output; no confusing capture drift with MCP support; no claiming a local or historical run certifies the final release commit.
