# Public source alpha launch packet

Proposed label: **v0.1.0-alpha.1**. Status: **private candidate; not approved or published**. Scope: source evaluation for Linux Docker servers and macOS developers. This packet prepares a reviewable launch; writing it does not authorize the steps below.

## Owner decisions required

1. Review the [dependency remediation and maintenance warning](../security/rsa-runtime-disposition.md). The old RSA package is removed; no advisory waiver is requested. Source-alpha scope remains experimental, with production/model/capture/distribution gates explicit.
2. Accept the [historical disclosure findings](../reviews/2026-10-12-public-alpha.md): older document versions retain a local username/project path, and Git author/committer names and email metadata remain visible. Current examples are generalized. Retained Actions logs also expose normal workflow/run metadata when public; the bounded log review and remaining refresh requirements are recorded in that report. If those exposures are unacceptable, choose a separately reviewed history-sanitization plan before publication; history has not been rewritten.
3. Approve integration of draft [PR #1](https://github.com/DharanesshMD/workspace-knowledge/pull/1), changing repository visibility to public, enabling private reporting and creating the source-only prerelease/tag **v0.1.0-alpha.1**. Approve each action explicitly. Do not upload the private unsigned binary candidates or local evidence directory.

The owner has confirmed monitoring GitHub private vulnerability reports after launch. Production sign-in, real-server/team testing and Apple signing remain deferred by the owner.

## Before integration

- Review the final candidate SHA and its exact-head checks, not an older branch/main result. All applicable foundation, history and dependency checks must pass at the exact final revision. Investigate any failure; do not waive or suppress an advisory. Do not bypass repository protection rules or describe a failing audit as green.
- Confirm [README](../../README.md), [CHANGELOG](../../CHANGELOG.md), [SECURITY](../../SECURITY.md), CODE_OF_CONDUCT, contributor templates, LICENSE/NOTICE and the publication review agree about scope and reporting.
- Confirm the fresh-source CLI demo and existing script checks, and refresh history scanning for any new commits. A token scan is only one part of the privacy review.
- Verify no captures, local credentials, generated database state, developer tooling or private evidence enter the merge or release assets.
- Inventory **all retained GitHub Actions artifacts and run logs**, including runs from old workflow revisions. Before public visibility, confirm no unsigned binary/frontend candidate or private evidence artifact remains: require separately authorized deletion or confirmed expiry if found. Review retained logs for private output and metadata, resolve any unreviewed runs, and refresh the inventory after final CI. Deleting an artifact alone does not remove its run logs. Do not silently delete remote evidence or assume Git secret scanning covers Actions.
- Confirm the candidate-build workflow's native/console jobs are restricted to private repositories while the alpha is source-only. Before visibility changes, disable that workflow in repository Actions settings and confirm no candidate-build run is queued/running; an old private-event run could otherwise upload after the visibility switch. The condition prevents later public dispatches, not an already-running private event. Re-enablement/binary distribution requires separately reviewed scope and artifact compliance.

## Approved launch sequence

These steps are manual and contingent on the owner decisions above. Stop if a new failure or unexpected content appears.

1. Integrate the reviewed candidate using the repository's permitted merge process. Update README clone instructions to the integrated default branch in that reviewed change; the current candidate branch instructions remain accurate until then. Check foundation/history results on the resulting main SHA and record the known dependency result separately.
2. Recheck all reachable history and retained Actions runs/artifacts before making it public. Confirm the owner-approved metadata exposure, resolved log review, absence of prohibited artifacts, disabled candidate-build workflow/no active candidate-build runs and license/asset inventory. Record the exact merge SHA; do not publish a different revision.
3. Change visibility in repository **Settings → General → Danger Zone** only with the explicit visibility approval. Verify the resulting repository is public. Public history can be copied immediately; reverting visibility cannot undo disclosure.
4. In **Settings → Advanced Security**, enable GitHub private vulnerability reporting. Verify **Security → Report a vulnerability** is available, the reporting policy is correct and the owner can access repository security advisories. Do not submit a fake report or contact others as a verification shortcut. If enablement fails, halt tag/release publication and tell the owner; SECURITY.md must state the actual available channel. Configure main-branch protection with the verified foundation/history/dependency check contexts after the public-repository features become available; the private main is currently unprotected.
5. With separate tag/release approval, tag the reviewed main SHA as `v0.1.0-alpha.1` and create a GitHub release marked **prerelease**. Use the unreleased changelog, advisory disclosure and support boundaries as release notes. Attach **no binary, capture or private evidence files**. GitHub's generated source archives are the alpha deliverable. Verify the tag resolves to the approved SHA and notes accurately disclose the audit result and remaining informational warning.
6. Record the resulting SHA/tag, reporting verification and URLs. Move the changelog entry out of Unreleased and update README status only after those actions succeed. Review any follow-up documentation change normally.

GitHub documents that [visibility changes expose Actions logs](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/managing-repository-settings/setting-repository-visibility) and [repository readers can download workflow artifacts](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/download-workflow-artifacts). These are part of the publication boundary even when no release files are attached.

GitHub's [private reporting documentation](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/configure-vulnerability-reporting/configure-for-a-repository) describes the public-repository feature. No security response SLA or new external account is assumed.

## Evaluation from source

While the repository is private, access is limited to authorized collaborators. Follow [quickstart](../../README.md#quickstart) for the current candidate or [installation](install.md) for a team evaluation. After the approved alpha tag exists, evaluators can clone that tag:

```sh
git clone --branch v0.1.0-alpha.1 https://github.com/DharanesshMD/workspace-knowledge.git
cd workspace-knowledge
cargo build --locked -p wk-cli
python3 scripts/demo.py
```

Install the pinned Rust toolchain/prerequisites first. This demo uses real CLI code and owned synthetic temporary state; it does not require a database, identity provider, model or real conversations. Server evaluation additionally requires PostgreSQL, separate configured portal/collector OIDC audiences and the documented deployment controls. The production provider/HTTPS flow has not been qualified.

## Later supported-release gates

See the [v0.1 checklist](v0.1-release.md). Required future evidence includes production OIDC/TLS, target-team/server and long-duration capacity, an expanded passing privacy corpus, approved capture versions, signed/notarized macOS distribution and artifact-specific license/restore acceptance. Source-alpha publication does not close those gates.
