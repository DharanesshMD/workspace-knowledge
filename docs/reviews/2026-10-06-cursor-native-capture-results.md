# Cursor native capture results

Updated 2026-10-07. Implementation in `codex/cursor-native-capture`; no production deployment or universal-client certification.

Implemented: immutable activity-session scope; shared submit/correct/publish sensitive-content checks; strict version-specific Cursor parser; explicit consent; descriptor-based no-follow source access; atomic encrypted session/checkpoint; fixed TTL and independent purge; native draft eligibility and source revalidation; overlap card revisions.

Verified so far:
- Security regressions failed before fixes and passed afterward: 15 PostgreSQL access tests, including non-owner RLS, plus four HTTP tests and existing HTTP/MCP integration.
- Parser, source, recovery, candidate and local storage tests exercise incomplete/error turns, distinct identical prompts, outside/unknown scope, symlinks, replacement, expired source, prefix mutation, encrypted restart/deduplication and closed-session guards.
- Deterministic end-to-end harness passed with native macOS Keychain, synthetic callbacks/transcripts and real PostgreSQL.
- The same harness passed with local Ollama `gemma4:e2b-mlx`. The generated summary preserved idempotency keys, retry limit of three, proposed lifecycle and the pending implementation review. It contained relative file evidence without a transcript path. A later safe turn recovered without another callback. Sensitive continuation held the source and prevented pending-draft submission.
- Separate API test using the actual locally generated card passed owner/reviewer/teammate/denied-service/other-organization authorization and overlap revision checks.
- Live Cursor 3.23.23 changed the synthetic file from 2 to 3, then stalled. Cancellation produced `turn_ended/status:error`. The source was held because of a previously unobserved timestamp wrapper; a regression reproduced this, then pinned support for that wrapper. The held original session was not reopened.
- Rebuilt collector access to existing test credentials triggered a macOS Keychain permission prompt; the user allowed it. Automated desktop control cannot interact with SecurityAgent. This is a native packaging/usability limitation, not evidence of unattended installation reliability.

Still in progress: fresh live session after the timestamp fix, no-tool turns, restart, revocation/outage checks, final checks and independent security review. These gates are not certified by the passing synthetic harness. The final result will record observed outcomes rather than replacing missing evidence with a success label.

No global hooks or real developer projects were enrolled. Tests operate in owned disposable folders and `/wk_test`. Shared summaries remain review-required. Unsupported clients, shell/MCP/attachments/compaction/subagents, entirely missed initial sessions, production scale and signed-release deployment remain outside this slice.
