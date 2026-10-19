# Production-Supported Launch Design

**Status:** Approved scope; implementation plan pending review.

## Goal

Prepare Workspace Knowledge for a production-supported public launch while leaving the GitHub repository private until the owner changes visibility manually.

## Supported boundary

- The initial production-support target is a Linux Docker server and macOS developer machines.
- Support claims name exact architectures, client versions, and client surfaces only after the corresponding install, workflow, and failure-mode acceptance passes. Other clients and platforms remain explicitly experimental or partial.
- Automatic publication remains disabled. Retrieval remains advisory; the product does not promise complete capture, safe edits, or reconstructed undocumented rationale.
- Production acceptance must use an explicitly selected OIDC provider, HTTPS deployment, representative real-team workflow, and measured target-server capacity. Synthetic/local acceptance alone does not close these gates.
- macOS distribution is production-supported only after signed/notarized artifacts, installation, upgrade/uninstall, rollback, and signature verification pass on a clean machine.
- Public launch claims require independent security review, clean exact-head automated checks, current source/history and Actions log/artifact review, and accurate docs.

## Release constraints

- Do not change repository visibility; the owner will do so manually.
- Do not enable automatic publication or broaden capture beyond explicitly accepted scopes.
- Never use real private conversations or production data as test fixtures.
- Do not claim a gate passed from code presence, synthetic tests, or stale reports; record evidence tied to the exact commit, deployment, OS, client version, and workflow.
- External gates requiring owner resources (provider/account selection, signing credentials, pilot infrastructure/participants, independent reviewer, or GitHub billing) remain open until completed and evidenced.

## Go/no-go criteria

1. Clean install through first successful team workflow is reproducible from current public-facing docs on the supported targets.
2. Production OIDC/HTTPS, least-privilege deployment, backup/restore, deletion, pause/revocation, and recovery are verified in the selected staging/target environment.
3. Approved client/version acceptance and the expanded privacy corpus pass; ambiguous, sensitive, unsupported, and failure cases remain held/private with clear user-facing explanations.
4. Signed/notarized macOS release distribution and rollback are verified.
5. A representative real-team pilot and long-duration capacity profile meet a published operational envelope.
6. Independent security review findings are resolved or explicitly dispositioned by the owner; exact-head CI and dependency/history checks pass.
7. README, SECURITY, changelog, setup, release checklist, and GitHub release metadata describe the same scope and current status.
8. Repository visibility remains private until the owner changes it manually.
