# Code change context: who, why, and what may be affected

Date: 2026-10-11
Status: approved by the user on 2026-10-11; not implemented or certified.

## Purpose and agreed behavior

When a developer pulls a teammate's code, their AI agent should receive useful context before changing it: who contributed the relevant code, the recorded reason for it, constraints to preserve, known consumers, relevant tests, and whom the developer can contact. Each factual statement must carry its source and scope. Missing information must appear as missing information, rather than an invented explanation or an assurance that the change is safe.

The user selected **require developer confirmation before proceeding when rationale or dependencies are missing, outdated, or high-risk**. This is the default for enrolled, supported edit workflows. It is a risk acknowledgement, not proof of safety, a grant of additional access, or authorization to message a teammate.

Success is a two-person workflow: Developer A records and confirms a reason for code at an identified revision; Developer B pulls that revision; B's agent receives A's appropriately verified name, A's confirmed rationale, and authorized impact references before its edit. B can inspect those references and contact A or the current maintainer. The application cannot guarantee that an arbitrary edit will never break a system, or force every third-party agent to consult an MCP tool.

## Approach selection

| Approach | Benefit | Limitation |
| --- | --- | --- |
| Join repository provenance with confirmed rationale and reviewed dependencies — recommended | Separates identity, intent, and impact; detects stale context; supports a meaningful before-edit gate | Requires explicit repository binding and small, reviewed rationale records; coverage must be measured |
| Git history alone | Immediately available in many existing checkouts | Last modifier is not necessarily the designer or maintainer; history does not establish the reason or complete dependencies |
| Generated summaries alone | Reuses current capture and search | Can misattribute intent, attach the wrong commit, or describe a proposal as implemented; not sufficient for a protection decision |

The first approach reuses the existing review/publication, authorization, evidence, and MCP boundaries. Generation can propose a concise explanation, but cannot establish a person's intent or promote an inferred dependency to fact.

## Current baseline and gap

The current application has approved local workspace capture, owner-only summaries, published team cards, service-scoped retrieval, overlap notices, reviewed graph relationships, and signed evidence ingestion. The inspected overlap adapter delivers Claude Code notices through prompt/edit hooks and Cursor notices after an edit. These notices are advisory and nonblocking. They do not constitute the proposed gate.

Current overlap results identify an activity reporter or card author by internal user ID, and match relative files, nearby folders, or text. A card author is not necessarily the author of code. File paths without repository identity cannot establish provenance across repositories. Signed evidence verifies its trusted producer; production repository/CI collection is a separate integration, not supplied by installation.

This design adds the missing join between an approved repository snapshot, contributors, confirmed rationale, and bounded impact references. Existing capture support does not imply before-edit protection support. See [overlap notices](../../architecture/overlap-notices.md), [backend boundaries](../../architecture/backend.md), and [signed evidence](../../architecture/evidence-connectors.md).

## Scope of the first increment

Include repository binding and bounded local Git observation; scoped contributor profiles and reviewed identity mappings; explicit rationale-to-code links; one deterministic change-context service; CLI/MCP retrieval; a hosted brief and human confirmation flow; and one version-pinned live before-edit adapter. Claude Code is the first integration candidate because its documented pre-tool interface includes denial and user interaction, subject to acceptance on the installed version.

The same brief can later support task start/resume, investigation reuse, and team coordination. Do not build a generic task manager first. Automatic full-repository indexing, production Git/CI provider connectors, framework-specific semantic analysis, automatic messaging, and additional protected agent clients require separate specs and acceptance. V1 uses reviewed records and bounded local observations, so it does not depend on selecting a hosted Git provider or production identity provider now.

## Developer workflow

1. The operator binds a stable repository ID to its organization/product and approved services. A developer enrolls the local checkout using the existing explicit workspace approval. A repository remote URL or a matching folder name never grants authorization.
2. A developer opts into a team-visible display name and optional approved contact link. The operator reviews contributor-identity mappings and their evidence. The UI distinguishes team-attested attribution from provider-verified attribution and unmapped Git metadata.
3. After work, capture may propose a rationale card. The author confirms the reason, constraints, and code anchors before publication. A reviewer checks scope and repository linkage. Legacy heuristic commit links remain candidates until reviewed.
4. Another developer pulls the code. Before a covered edit, the local agent observes the target checkout revision and relevant file state, and requests a change-context brief. Explicit CLI/MCP retrieval is also available without protected hooks.
5. The brief cites confirmed rationale and contributor roles, lists known authorized consumers and tests, and identifies gaps. A missing or stale reason, incomplete dependency coverage, recorded conflict, or high-risk target requires human confirmation before a covered edit.
6. The developer inspects the brief, revises the plan, contacts a contributor themselves, or acknowledges the specific risk. The acknowledgement is bound to this operation and snapshot; it is not a session-wide bypass.
7. After an edit, the client checks the actual touched paths and reports coverage deviations locally. Relevant tests are suggested. Only matching trusted test evidence can establish verified status. New rationale starts as a draft and goes through existing publication review.

Illustrative output, using fictional people and sources:

```text
Before changing api/orders/retry.ts at repository orders / commit abc123…
Contributor: Priya Shah — team-attested contribution at this commit/range.
Recorded reason: Priya confirmed that retries must preserve the operation ID
to prevent duplicate charges. Source: published decision D-42, revision 3.
Current maintainer: Mateo Ruiz — explicit maintained-by record, not inferred.
Known impact: billing-worker relies on that ID. Source: reviewed dependency E-8.
Relevant check: duplicate-charge contract test. Result for this revision: unknown.
Coverage: reviewed dependency records only; runtime consumers may be missing.
Decision: developer confirmation required. Suggested question for Priya:
"Does the duplicate-charge constraint still apply to this retry change?"
```

The UI shows the actual source assurance and freshness, not the fictional labels above. It must not describe Priya as the original designer solely because blame names her, or claim the recorded constraint is implemented merely because it is published.

## Components and responsibilities

| Component | Responsibility | Depends on |
| --- | --- | --- |
| Local repository observer | Establish approved checkout, immutable base revision, relevant ranges and local changes; extract bounded provenance | Existing workspace/device approval, safe read-only Git invocation |
| Attribution records | Join observed contributor identities to scoped, reviewed profiles; keep authorship separate from maintainership | Existing internal identity/membership, operator authority, mapping evidence |
| Rationale anchors | Attach published, author-confirmed statements to explicit repository/revision/file/range anchors | Existing card review, revisions, publication and withdrawal |
| Change-context assembler | Return source-backed rationale, known impact, freshness and required review reasons | Authorized anchors, reviewed edges, current policy; no private summary retrieval |
| Local protection adapter | Bind the brief and human acknowledgement to a covered edit; recheck local state before dispatch | Version-specific hooks, authenticated local IPC, assembler response |
| Hosted brief | Show names, sources, gaps and human acknowledgement | Existing browser authentication and same service-scoped API contract |

Extend these existing boundaries with focused modules; do not introduce a new graph database, vector store, background LLM service, or general orchestration subsystem for this increment.

## Repository identity and safe observation

All code anchors use `(org, product, repository_id, service scope, immutable revision, relative path, optional range)`. A path alone is never a factual join key. Operator-controlled repository registration has a revision; the local approved binding records its ID and authorized Git metadata roots. Worktrees can share an explicitly enrolled Git metadata directory; submodules and other repositories require independent binding. Credential-bearing remote URLs are neither exported nor trusted as identities.

Resolve HEAD to a full immutable object ID and observe target ranges at that revision. Bind local unstaged/staged modifications to a local content digest. The digest and edit-input fingerprint remain local; do not upload code, patches, raw tool inputs, raw prompts, commit messages, absolute paths, Git emails, or unscreened author names. Server lookup receives approved relative paths, the base revision, and opaque local snapshot/operation handles. Requester-supplied observations are labelled local observations, not trusted repository evidence.

For V1, use exact file anchors at an identical revision; optional reviewed line ranges improve targeting only when their anchor still matches. Changed revisions or overlapping dirty ranges require review. Unaffected anchors at another revision may be offered as historical candidates, not automatically certified current. Rename/copy detection, shallow history, merges, generated files, binary files, untracked files, or missing target ranges must expose the applicable uncertainty. Never silently substitute latest main for the developer's checkout.

Git runs with fixed arguments, no shell interpolation, a validated executable, and read-only operations. Disable executable external diff/text conversion, credential helpers/network requests, replacement-object interpretation, and unneeded config behavior. Bound subprocess time, output, history/ranges and target count. Validate canonical paths, Git directories and symlinks before reads; reject escape into an unapproved repository. Invalid or unreadable targets do not yield an all-clear result.

## Names, attribution and contact

Keep five distinct roles in the output: observed code contributor, confirmed rationale author, published-card submitter, current maintainer, and activity reporter. Multiple contributors are allowed; no single file-owner guess. Git commit author and committer fields are self-asserted metadata. A signed commit can strengthen provenance without itself proving the mapping to an application user or responsibility for the whole design.

A scoped profile contains internal identity, opted-in display name, optional contact link, service visibility, profile revision and revocation state. Identity mapping stores repository/provider identity, assurance class, reviewed proof reference, reviewer and mapping revision. Provider integrations should use immutable provider account identifiers, not matching names or emails. V1 supports explicit operator-reviewed mappings with developer confirmation; label these **team-attested**, not provider-verified. Git email/name matching may suggest a mapping privately but cannot publish or verify it automatically.

Mapping an identity does not authenticate every commit that uses that name/email. V1 also requires a contribution attestation linking the authenticated developer to the specific repository/commit/targets, confirmed by that developer and reviewed for publication. Until then, even a matching mapped author field remains an unconfirmed Git assertion. A future provider connector must attest the actual commit/contribution association, not merely verify that an account exists. This prevents a forged Git author field from acquiring a teammate's trusted name through a valid account mapping.

Export only approved profile identity references from local observation. If mapping is absent or disputed, return `unmapped contributor`; do not expose raw email or invent a developer name. A rationale written by a different person must be labelled that person's report unless the asserted intent author confirms it. An authenticated developer can confirm their own recorded intent; this confirms the statement's attribution, not code correctness. Current maintainers require a separate reviewed, revisioned assignment.

Names/contact details appear only with evidence the requester may access and within profile service visibility. No unrestricted member directory, hidden-member counts or cross-product identity lookup. Departed or revoked contributors may retain an explicitly approved historical attribution on already-published evidence; current membership and contact availability are not implied. Profile removal suppresses contact and display details without resurrecting withdrawn cards. Contact URLs must be credential-free HTTPS on an approved host list. The application may draft a question locally; it does not send it.

## Rationale, impact and freshness

A rationale anchor references an existing card ID/revision plus repository ID, immutable code revision, relative targets, optional ranges, intent-confirming identity/time, contribution-attestation reference where authorship is claimed, and reviewed linkage evidence. Confirmation attaches to the exact statement revision. Publication confirms visibility and review, not implemented/verified lifecycle. Corrections, supersession, withdrawal, identity-map changes, repository rebindings and source revocations invalidate the affected joins and outstanding briefs.

Do not backfill existing heuristic file/commit observations into confirmed anchors. The author/reviewer sees candidates and explicitly validates them. Unknown reasons are displayed as `reason not recorded`. Contradictory records are displayed with their source revisions and require review. Derived summaries cannot strengthen the assurance of their sources.

Impact V1 consists of explicitly reviewed `depends_on`/conflict/supersession records between authorized, anchored cards, recorded interface constraints, and available revision-matching test references. Include the caller's own published records as well as other developers' records. Distinguish exact range, exact file, historical candidate, nearby file and semantic similarity; nearby/text matches may help discovery but do not establish authorship, rationale applicability or a dependency.

Report dependency coverage as `reviewed records only` and therefore incomplete in V1. Absence of returned dependencies is not evidence of no consumers. This means initial protected edits normally require acknowledgement even when a reason is recorded; this is the user's selected safety behavior, not a defect to hide. Framework-specific static analysis and production CI evidence can improve the brief later, but an analyzer must publish its supported boundary and still disclose dynamic/runtime gaps.

Every source includes its ID/revision, kind, immutable code reference when available, observed/confirmed time, current freshness and attribution assurance. A trusted producer signature is evidence of its origin, not infallibility. CI results are current only for the matching code revision and evidence boundary. Local agent-reported test results remain reported. Activity is phrased as **last reported working on** with a timestamp, not proof that someone is currently editing or responsible for the code.

## Retrieval contract and authorization

Add a versioned `POST /v1/products/{product}/change-context` and MCP `knowledge.change_context`, preserving existing `knowledge.context` and `knowledge.overlaps` contracts. A local CLI offers the same brief. CLI observations are local assertions; arbitrary MCP clients without a bound device can retrieve authorized references but cannot obtain an enforceable local protection receipt.

Request fields: registered repository ID, full base revision, bounded approved targets/ranges, operation class, opaque local operation/snapshot handle, authenticated device/binding identifier for protected mode, and output budget. Do not require a natural-language prompt or full proposed patch. Authentication determines user, org and permissions; body claims cannot override them. Default bound: 20 targets, 20 cited records, 64 KiB request and 16 KiB response; exceeding bounds yields `needs_review` or rejected input, not silently complete coverage. Safety/gap indicators must survive response truncation.

Response fields: observation assurance, repository/snapshot anchors, source revision set, contributor roles, confirmed rationale, recorded constraints, authorized known impacts, test references, last-reported overlaps, coverage/freshness gaps, decision/reasons, and an expiring opaque brief ID. The narrative is rendered deterministically from those fields. No LLM is required in the pre-edit decision path. An LLM may interpret references but cannot sign a decision, author acknowledgement, or change assurance labels.

Enforce org/product and every current service permission on repositories, anchors, profiles, edges, contacts and source citations, using explicit queries plus RLS. An anchor is as restricted as its card, repository and relevant profile. Do not access owner-only summaries, unreviewed capture, raw journals or credentials. Do not traverse a visible edge into a hidden service, expose its label, or reveal counts of hidden results. State coverage is incomplete generically, without revealing whether hidden records exist. Auth denial is never converted into an overridable risk acknowledgement.

Source links are authorized same-origin detail routes or credential-free allowlisted references. Do not automatically fetch arbitrary URLs, execute embedded instructions, or promote source text into system instructions or project settings. Profile strings and summaries are escaped as untrusted data in the portal and labelled reference material in agent output.

## Confirmation gate and capability limits

Separate capture from protection. Existing capture failures stay nonblocking. Developers explicitly enroll supported edit workflows in protection; the selected default requires confirmation for unknown, stale, conflicting or high-risk context. Authorization errors and out-of-bound paths are denied and require correcting configuration, not clicking through.

V1 decision states:

- `context_available`: citations retrieved; coverage still disclosed; never synonymous with safe.
- `needs_review`: missing/stale intent, incomplete dependencies, conflicting constraints, active overlap, test gaps relevant to a recorded requirement, or registered high-risk scope. No covered edit proceeds before the developer acknowledges this exact brief.
- `denied`: unauthorized or invalid scope. Acknowledgement cannot permit it.
- `unavailable`: deadline/network/freshness failure. In protected mode treat as needing human review; show what could not be checked. No silent allow from an empty/error response.

High-risk scopes are operator-reviewed repository path rules for shared interfaces, migrations and similar team-defined boundaries; a model does not secretly invent policy. V1 has no offline protected-edit override: hold the covered edit when current authorization/source validity cannot be checked, show the unavailable reason, and obtain a fresh brief plus human confirmation after recovery. Locally cached context can remain advisory but cannot authorize the write. Explicitly leaving protected mode is a separate user-controlled configuration change and must be visibly labelled uncovered, not a successful preflight.

Human confirmation uses the authenticated developer browser session or the host's verified human interaction route; acknowledgement is never exposed as an MCP/collector tool. Agent text such as "the user approved" and MCP tool arguments cannot generate acknowledgement. A browser confirmation is bound to the requester, enrolled device, brief ID, exact source revision set, policy revisions and operation handle, with CSRF protection. The local agent binds the operation to its full edit-input fingerprint and target-file digests without sending those contents to the server. Acknowledgement is single-use and expires after five minutes, as does the brief. It authorizes only the declared edit scope.

The local threat model is a cooperating agent and trusted desktop/client, not a compromised process sharing the developer's OS account or browser session. An agent with desktop automation or access to browser credentials may impersonate UI interaction; ordinary web authentication alone is not proof of human presence. Such a surface cannot claim independent human confirmation unless a host/OS mechanism outside that agent's control enforces it. Certification must document the actual interaction trust boundary and reject broader claims. Preventing a malicious local agent from editing files or disabling hooks requires OS isolation beyond this increment.

Immediately before dispatch, the local adapter validates the snapshot, operation fingerprint and current authorization/source validity; changed HEAD, target content, targets, repository binding, membership/policy, source revision, withdrawal or mapping revocation requires a new brief/acknowledgement. The server serializes validation against relevant policy/source changes; the client rechecks files after the lookup. Network partitions become `unavailable`, not validated permission. A native hook still has a small race between returning and the host writing; report that limit. Atomic exclusion of external writers requires a future managed edit executor, not a claim that a hook locks the filesystem.

Certification records client, version, OS, tool names, pre-write behavior, confirmation route, timeout/crash behavior and known bypasses. A supported hook must demonstrably stop the write until human acknowledgement; simply adding context, denying once then letting a retry pass, or asking the agent to obey a rule is insufficient. If an actual client timeout/crash bypasses the gate, that version cannot be labelled protected.

Claude Code is the initial live acceptance target. Current documentation describes pre-tool decision hooks, but test the installed version rather than assuming current docs match it. Cursor documents a pre-tool deny response; evaluate it separately from the existing after-edit notice adapter. Codex and Antigravity get explicit retrieval/advisory coverage until a tested pre-write integration is available. Native conversation capture certification does not certify edit interception.

Protection labels apply only to named, intercepted tools. Shell scripts, other MCP writers, manual IDE edits, unsupported tool input shapes and disabling hooks can bypass a narrow integration. V1 must expose these limits and must not call the entire repository protected. A managed workspace that forbids all uncovered writers would be a separate product mode. Post-edit observation can reveal a deviation, but cannot undo harm or be advertised as preventing it.

## Storage, bounds and operations

Use PostgreSQL for scoped repository registrations, profiles/mappings, rationale anchors, maintainer assignments and short-lived brief/acknowledgement records. Extend the existing SQLite/local agent state for observation cache and single-use edit fingerprints. Keep raw conversation retention unchanged. Invalidate caches by source/policy/profile/repository revisions; no positive offline freshness cache.

Index authorized anchors by repository/revision/path, and references by card/revision. Bound graph traversal to one reviewed dependency hop and at most 20 references in V1; disclose truncation. Candidate searches and Git observation have a two-second combined interactive target and a five-second hard request deadline. These are proposed targets, not measured guarantees; timeout requires review. No full history scan or model generation on each keystroke.

Keep brief/acknowledgement payloads for at most 24 hours, clear them after expiry through bounded retention work, and preserve only minimal decision audit metadata under the existing configured audit retention. Digests/edit inputs remain local and expire with the receipt. Audit user/device, opaque brief ID, decision, acknowledgement time and policy revision, without raw code, names/emails, contact links, prompts or conversation IDs. Restores must respect withdrawals/revocations and expire all old acknowledgements; no reusable approval emerges from a backup.

Benchmark concurrent pre-edit checks for five developers with cold and warm caches and large/shallow repositories. Record latency, DB growth per anchor/brief, deadline behavior and missed/bypassed tools. Existing portal load tests are not evidence for this new path. Prefer bounded indexed queries and ephemeral records over duplicating each developer's repository in the hosted database.

## Acceptance criteria

| Scenario | Required result |
| --- | --- |
| Two distinct developers, exact revision | A confirms and publishes rationale; B pulls it and receives A's approved name, role, statement and citations before a covered edit. No raw conversation is available to B |
| Different summary author and code contributor | Roles and statement attribution remain distinct; submitter is not renamed as code author |
| Same path in two repositories | No cross-repository rationale or identity join |
| Forged Git name/email or ambiguous identity | No trusted contributor assignment, even if the forged field matches an existing mapping; require a revision-specific attestation |
| Multiple authors, departed contributor, explicit maintainer | Roles and historical assurance are preserved; unavailable contact is explicit; no inferred current membership |
| Missing rationale or dependency coverage | Explicit gaps and required developer acknowledgement; no invented reason or all-clear |
| Source revised, withdrawn, contradicted or scoped away | Prior brief invalidated; no reuse of acknowledgement or hidden source details |
| Pull/rebase, dirty target, new target or changed edit input | Snapshot mismatch and new preflight; acknowledgement cannot cover the new operation |
| Proposed decision or local test report | Does not become implemented/verified merely through publication or developer confirmation |
| Scoped access, profile service limits, private summaries | Every API, MCP, portal and citation path enforces scope; no private bodies, hidden counts or contact directory leakage |
| Real supported client before-write test | An intentional risky edit does not modify the file before trusted human confirmation; retry, timeout, error and unsupported input cases tested |
| Unsupported client/tool or bypass writer | Coverage visibly advisory/uncovered, never an assertion that a write was prevented |
| Offline/failing server | Protected path holds the edit until fresh authorization/context and required human acknowledgement after recovery; capture remains independent |
| Malicious source/reference/repository metadata | No command execution, URL credential leak, path escape, secret/code upload, or promotion of reference text to instructions |
| Test evidence and actual patch | Suggested checks tied to known constraints; code/test evidence must match revision before verification; unexpected writes are reported |
| Confirmation forgery, replay and restore | Agent-supplied approval rejected; receipts single-use/expiring/revision-bound; restored acknowledgements invalid; human-presence claims match the real host/OS boundary |

Use a disposable repository, database and two separately provisioned synthetic developer identities for end-to-end acceptance. Include an actual pulled commit, confirmed rationale, dependent consumer and intentionally unsafe edit. Distinct sessions of one identity are not a substitute for testing attribution across developers. Do not claim production readiness based only on mock hook payloads.

## Delivery boundary and review

The implementation plan following approval should order this increment as: attribution and explicit anchors; bounded observer and assembler; CLI/MCP/hosted brief; human acknowledgement; one client adapter; multi-developer live acceptance and documentation. An increment is useful for retrieval before it is protected, but must be labelled accordingly.

Release claims must say which repositories, services, evidence sources, clients, versions and tools were tested, which gaps remain, and whether confirmation enforcement or only advice is available. Existing capture limitations, production provider configuration and known dependency/security release gates remain independent. This spec does not authorize publishing a release or contacting contributors.

## Primary references

- [Git blame documentation](https://git-scm.com/docs/git-blame): line attribution reports the revision/author that last modified a line; deeper history is needed for other provenance questions.
- [Claude Code hooks reference](https://code.claude.com/docs/en/hooks): pre-tool decisions and their client-specific interaction behavior; installed-version acceptance remains required.
- [Cursor hooks reference](https://cursor.com/docs/hooks): pre-tool allow/deny interface; separate certification is required from current after-edit delivery.

References checked 2026-10-11. They establish integration possibilities, not that this application already implements or enforces them.
