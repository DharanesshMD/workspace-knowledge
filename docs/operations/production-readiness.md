# Production readiness

**Status: blocked — not production-supported.** This ledger records the repository and release state observed on 2026-10-19. The source-alpha baseline `main` commit is `5605d788cd401997aa8ee06706cb6c6ff8e8e6ef`; readiness edits remain uncommitted, so there is no final release SHA. This is a readiness snapshot, not a claim that the open gates have passed.

## Current repository and release state

| Item | Observed state |
|---|---|
| GitHub visibility | Private. The owner will change visibility manually; no agent workflow changes it. |
| Latest GitHub release | `v0.1.0-alpha.3`, source-only prerelease published 2026-10-17; no release binaries attached. |
| Alpha tag | `v0.1.0-alpha.3` resolves to commit `5b982a5f4523d1e017de4d190586c715538abbe9`. |
| Current `main` at audit | `5605d788cd401997aa8ee06706cb6c6ff8e8e6ef`, ahead of alpha.3; includes post-tag model-free summary changes. |
| Retained Actions artifacts | GitHub artifact API returned `total_count: 0` on 2026-10-19. Recheck after final CI because artifacts and workflow logs have different retention. |
| Exact-head GitHub CI | Foundation and dependency workflows for `5605d78` failed before any job step ran: GitHub reported account payment failure or spending limit. This is not a code-test result. Resolve billing and rerun at the final SHA. |
| Local verification | On the uncommitted working tree based on `5605d78`: full Rust workspace tests passed (database-backed tests requiring a disposable PostgreSQL were ignored); `cargo fmt --check`, strict Clippy, 24 Python tests (one optional Gitleaks-dependent test skipped), the web production build, Compose validation with synthetic-only placeholders, and the Linux server image build passed. Existing portal Playwright tests passed 38/38 against synthetic APIs with desktop/mobile views visually checked. A local Gitleaks scan of touched source/release files found no leaks; published `main` history was clean. These checks do not replace exact-final-SHA remote CI, database integration, provider, or real-team acceptance. |
| Dependency audit | Local `cargo audit` on 2026-10-19 reported zero vulnerability findings and four allowed informational unmaintained-dependency advisories: `rustls-pemfile`, `zbus_names`, `zvariant`, and `zvariant_derive`. Treat this as maintenance exposure, not certification; see [SECURITY](../../SECURITY.md) for current RustSec references. |
| macOS signing identity | `security find-identity -v -p codesigning` returned zero valid identities. No signed/notarized release workflow or Gatekeeper acceptance can be qualified here. |

## Production go/no-go gates

| Gate | Status | Required evidence / owner input |
|---|---|---|
| Production identity and HTTPS | **Open — owner declined provider/staging selection** | Owner selects an OIDC provider and disposable staging tenant. Verify PKCE/state/nonce, portal and collector audiences, issuer/key rotation, expiry/logout, HTTPS reverse-proxy behavior, cookies, and operator recovery on the selected provider. A local synthetic provider is not production evidence. |
| Privacy and capture | **Open** | A six-sample local Qwen corpus now passes after common contact patterns are held before inference ([evidence](../reviews/2026-10-19-privacy-acceptance.md)). This is not privacy certification. Expand the multilingual/personal-data corpus and qualify exact client/version/surface on isolated profiles. Keep ambiguous, unsupported, and partial sessions held/private; automatic publication stays disabled. |
| macOS distribution | **Open — no signing identity available** | Owner supplies an approved Apple signing/notarization identity and protected credential path. Verify signed artifacts, install/upgrade/uninstall, Gatekeeper/signature state, license/notice contents, and exact rollback on a clean supported Mac. The local manifest refuses symlinks before output, but checksums do not authenticate publisher identity. |
| Target deployment and capacity | **Open — no pilot scope/budget supplied** | Owner defines pilot workload, duration, success thresholds and budget before infrastructure is provisioned. Run target-like Linux Docker/PostgreSQL workload and restore/deletion/recovery drills, then a consented real-team pilot. Publish only the measured operating envelope. No deployment or pilot has been provisioned. |
| Security assessment | **Open** | Obtain independent review, disposition findings, refresh historical disclosure and retained Actions-log review, and confirm actual vulnerability-reporting channel. The old RSA runtime package was removed; this and local audit results are not comprehensive security certification. |
| Exact-head CI and supply chain | **Blocked on GitHub billing** | The 2026-10-18 Foundation (`39126750755`) and Dependency audit (`39126750725`) runs target `5605d78`, contain zero runner steps, and have no job logs; GitHub reports account payment/spending-limit failure. Artifact API returned zero retained artifacts on 2026-10-19. Resolve billing; pass foundation, dependency, history/secret, container and release checks at the final commit. Do not delete remote evidence. |
| User-facing setup and support claims | **In progress** | Keep README, changelog, SECURITY, install, completion matrix, release checklist and GitHub release notes consistent. Reproduce clean install through the first supported team workflow and retain exact environment/client evidence. |
| Repository visibility | **Owner action only** | Keep private until the owner manually changes it. Public visibility does not make the application production-ready. |

## Release boundary

The proposed production-supported target is Linux Docker server plus macOS developer machines. Exact architectures and client surfaces must be named only after acceptance. Other operating systems and unqualified adapters remain experimental or partial. Do not promise universal capture, complete context, safe edits, or production security certification. Ordinary retrieval remains advisory.

Before any supported production release, close every gate above, rerun the complete local and remote verification at one exact final commit, and update this ledger with dated evidence and the final SHA. Historical alpha.3 remains a source-only prerelease; do not retag it or imply it contains changes made after its tag.

See [v0.1 release gates](v0.1-release.md), [historical alpha launch packet](public-alpha.md), [implementation plan](../superpowers/plans/2026-10-18-production-supported-launch.md), and the [approved launch scope](../superpowers/specs/2026-10-18-production-supported-launch.md).
