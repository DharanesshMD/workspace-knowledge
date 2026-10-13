# Public source alpha preparation review

Date: 2026-10-12. Baseline: `9a0099487b0c6725c1ec9695e0de76c22a95b3c7`, private `codex/v0.1-readiness` candidate. Proposed deliverable: source-only `v0.1.0-alpha.1`, Linux Docker server + macOS developers. No public visibility, merge, tag, release or binary publication is authorized by this record.

## Publication inventory

- Git's all-ref reachable-object inventory contained **1,498 named objects** at the baseline. Extension/type review identified **22 distinct historical PNG blobs and two JSONL fixture blobs**; no database dump, captured archive or binary-release blob was identified by this bounded type review.
- All 22 PNGs had no text/EXIF PNG chunks. Ten current unique image objects had been visually reviewed in the earlier readiness work and are unchanged at this baseline; the additional twelve historical-only images were freshly viewed at original resolution. Screenshots show synthetic Alice/Bob, test products, local ports, ephemeral installation identifiers and documented placeholders. Cover/avatar were generated brand artwork. No real conversation or credential was identified in these reviewed images. This does not establish that arbitrary bytes or every document are free of private information.
- The two JSONL fixtures contain synthetic Cursor retry dialogue and explicitly synthetic Antigravity hook-delivery tests. They are test data, not instructions or captured team conversations.
- Four historical document blobs in three paths retain a local developer username/project path: the hosted-operations execution ledger and the native-capture/code-change implementation plans. Current examples are generalized in this increment. Old objects still exist; no history was rewritten. Git history also retains one distinct author identity and one committer identity (names/emails). **Owner approval of that disclosure or a separate sanitization decision remains required.**
- `.gitignore` excludes target output, local toolchains, the plan workspace, environment files, databases, generated web output and key files. Ignore rules are not a retrospective guarantee. Full-history secret scanning and exact synthetic exceptions are separately documented in the [readiness record](2026-10-12-v0.1-readiness.md).
- Repository issue attachments, external chat archives and non-repository account data were not inspected. This review covers repository source/history and owned synthetic fixtures only. No live credential was tested, disclosed or rotated.

## GitHub Actions publication boundary — 2026-10-13 follow-up

Read-only repository API inventory found **zero Actions artifacts**, with **40 workflow runs**, of which 38 were completed and two exact-head checks were in progress. All 38 completed run-log archives were downloaded into owned private scratch (286 text files, about 10.87 MB). A default-rule redacted Gitleaks scan found no credential findings. Bounded output/metadata review found conversation/transcript references in synthetic test names/status/assertion messages, no JSON prompt/message-field dumps, no personal local paths or email matches; credential environment labels were reviewed without disclosing values. These checks are not proof of exhaustive privacy absence. Normal GitHub actor, repository, run and hosted-runner metadata will be public; owner disclosure approval remains required.

The fresh final reviewer identified an Important omission: Git history/release attachment checks did not cover retained Actions artifacts/logs. The manual candidate workflow could upload unsigned native binaries and built frontend assets. Both candidate jobs now require a private-repository event; a declarative YAML guard check failed before the fix and passed afterward. The launch runbook additionally requires disabling that workflow and ensuring no active/queued old private-event candidate run before visibility, checking all retained artifacts/logs, and obtaining authorization for any needed deletion. No remote evidence was deleted and no candidate workflow was dispatched.

The two in-progress runs and any later runs still require log review after completion; artifact inventory must be refreshed immediately before approved visibility changes. Zero artifacts at this snapshot is not a permanent guarantee.

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

## Independent review and bounded correction

On 2026-10-13, a fresh read-only reviewer found one Important Actions-publication omission, no Critical or Minor findings. The fix added the private-only candidate jobs and pre-visibility artifact/log/quiescence gate described above. The declarative guard failed before the change (both jobs unrestricted) and passed after; the eight existing Python tests passed again, all 107 changed Markdown file targets resolved, GitHub YAML parsed and diff checks passed. The first reviewer attempt had stopped at an account usage limit without producing a review; the successful retry is the actual independent review.

No new runtime code, broad secret exception or RSA audit suppression was introduced. Exact-head CI and final Actions/reporting verification are assessed in the draft PR and launch sequence rather than copied from an older commit. Preparation is ready for the owner's scoped decision after those checks; the repository remains private and unmerged.

## Final CI negative-control correction

Exact-head CI at `f7a7b1b` passed the full history scan but failed its merge-only credential negative control: the randomly generated canary was not detected. The original random value was not retained, so its exact skip reason cannot be reconstructed. Pinned Gitleaks 8.30.1 has an entropy cutoff and stopword/example filters; a valid low-entropy 48-character hex RNG output reproduced the failure locally. A stable nonfunctional SHA-derived 48-character fixture (measured entropy about 3.81) was detected by the same scanner/traversal. The test now uses that deterministic fixture, with no rule/entropy/audit relaxation or new exception. The controlled low-entropy RNG scenario failed before and passed after; the eight-test suite and CI must verify the final revision.
