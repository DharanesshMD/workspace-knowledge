# Public source alpha preparation review

Date: 2026-10-12. Baseline: `9a0099487b0c6725c1ec9695e0de76c22a95b3c7`, private `codex/v0.1-readiness` candidate. Proposed deliverable: source-only `v0.1.0-alpha.1`, Linux Docker server + macOS developers. No public visibility, merge, tag, release or binary publication is authorized by this record.

## Publication inventory

- Git's all-ref reachable-object inventory contained **1,498 named objects** at the baseline. Extension/type review identified **22 distinct historical PNG blobs and two JSONL fixture blobs**; no database dump, captured archive or binary-release blob was identified by this bounded type review.
- All 22 PNGs had no text/EXIF PNG chunks. Ten current unique image objects had been visually reviewed in the earlier readiness work and are unchanged at this baseline; the additional twelve historical-only images were freshly viewed at original resolution. Screenshots show synthetic Alice/Bob, test products, local ports, ephemeral installation identifiers and documented placeholders. Cover/avatar were generated brand artwork. No real conversation or credential was identified in these reviewed images. This does not establish that arbitrary bytes or every document are free of private information.
- The two JSONL fixtures contain synthetic Cursor retry dialogue and explicitly synthetic Antigravity hook-delivery tests. They are test data, not instructions or captured team conversations.
- Four historical document blobs in three paths retain a local developer username/project path: the hosted-operations execution ledger and the native-capture/code-change implementation plans. Current examples are generalized in this increment. Old objects still exist; no history was rewritten. Git history also retains one distinct author identity and one committer identity (names/emails). **Owner approval of that disclosure or a separate sanitization decision remains required.**
- `.gitignore` excludes target output, local toolchains, the plan workspace, environment files, databases, generated web output and key files. Ignore rules are not a retrospective guarantee. Full-history secret scanning and exact synthetic exceptions are separately documented in the [readiness record](2026-10-12-v0.1-readiness.md).
- Repository issue attachments, external chat archives and non-repository account data were not inspected. This review covers repository source/history and owned synthetic fixtures only. No live credential was tested, disclosed or rotated.

## Source, assets and fixture rights

Application source has the root Apache-2.0 LICENSE and NOTICE. CONTRIBUTING requires Apache-2.0 contributions and DCO sign-off. Generated cover/avatar provenance and prompts are recorded in [brand assets](../assets/README.md); screenshots originate from owned synthetic portal fixtures. No borrowed vendor logo was identified in those reviewed assets. Generated artwork provenance does not constitute an independent legal or trademark opinion.

The prior locked Cargo metadata inventory found declared license expressions for all packages, and the private curated macOS candidate retained dependency license texts/notices and an inventory. That inventory includes development/build dependencies and is not a precise linked SBOM. Direct frontend runtime dependencies React/React DOM declare MIT in the committed lockfile. Source alpha keeps LICENSE/NOTICE and lockfiles; packaging binaries or built frontend/container artifacts needs its own retained transitive license texts and distribution review. No binary assets are proposed for this alpha release.

Two committed RSA fixture files are explicitly synthetic, permanently public test keys. Their consumers are authentication/native-login tests and disposable identity/load harnesses. Never trust these keys in a deployment. Scanner exceptions are exact reviewed fingerprints/literals, not broad directory exemptions.

## Security and support decisions

The [runtime RSA assessment](../security/rsa-runtime-disposition.md) records a bounded independent trace through public-key OIDC verification. RUSTSEC-2023-0071 is still a normal runtime advisory with no demonstrated patch; the dependency gate remains red. A source-only alpha disposition is **proposed, pending explicit owner approval**, and no audit suppression is introduced.

The owner has confirmed that they will monitor GitHub private vulnerability reports after public launch. GitHub private reporting is not yet enabled on this private candidate; the launch runbook requires enablement and verification before tag/release publication. No response SLA or security email is invented.

Local acceptance covers the hosted portal and enrolled Claude Code 2.1.296 Edit/Write flow with synthetic fixtures. MultiEdit/cloud behavior and universal capture are unqualified. The corrected strict Qwen privacy corpus failed its contact-sensitive case, although application screening held that observed output. Automatic publication remains disabled. Production OIDC/TLS, target-server/team and longer capacity testing, Apple signing/notarization and supported binary distribution remain open. See the [continuation evidence](2026-10-12-v0.1-continuation.md).

## Verification of this increment

Fresh tracked-source export contained no local `.tooling`, target output, Git state or symlinks. The real pinned Cargo 1.98.1 built `wk-cli --locked` in that export using the installed toolchain/cache outside the source tree; the actual CLI consent/scope/replay/pause demo passed with synthetic temporary state. Eight existing Python tests passed from the fresh export, including the merge-only credential rejection using checksum-verified Gitleaks 8.30.1.

All 107 local Markdown file targets in changed documents resolved. Four GitHub YAML files and both issue-template frontmatters parsed with the installed Ruby YAML parser (the bundled Python runtime did not have PyYAML). The full-history redacted scan passed: 78 commits, approximately 4.11 MB, with the existing exact synthetic exceptions unchanged. The staged-diff redacted scan also passed (approximately 30 KB). `git diff --check` passed. These scans precede the final candidate commit; exact-head history/CI results must be checked after pushing. Prior acceptance counts are evidence for the unchanged runtime baseline, not fresh reruns of every platform or production qualification. The [launch packet](../operations/public-alpha.md) is the decision checklist; preparation does not make the candidate publicly released or all-tested.
