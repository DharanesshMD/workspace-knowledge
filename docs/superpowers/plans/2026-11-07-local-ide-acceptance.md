# Local Codex and Antigravity acceptance

Goal: configure the installed clients, test real model-driven sharing and retrieval with synthetic projects, and fix reproduced integration failures.

Scope: existing local Linux-container server; Codex/ChatGPT as ravi and Antigravity IDE as anika. Preserve existing client configuration and use private backups. Never import real conversations or bypass native capture qualification.

- [x] Back up client configuration, connect both clients through the existing authenticated MCP proxy, and confirm tool discovery.
- [x] Have Antigravity change a synthetic retry implementation and explicitly share its rationale and file list.
- [x] Have Codex retrieve the other developer's work by name and file, respect the recorded constraint, make and test a follow-up change, and share on request.
- [x] Retrieve the follow-up from Antigravity; exercise unknown names, empty results, isolation, and sensitive-content rejection.
- [x] Reproduce integration/configuration bugs before fixes; retain focused regressions for product logic and verify the relevant suites.
- [x] Record exact client versions, observed calls/results, changes, and remaining capture limitations in a dated review.

Known issues to verify: Compose drops the publication-policy environment setting; install documentation describes an older schema; native Codex and Antigravity capture targets differ from the installed clients.

Outcome: see [dated acceptance and verification evidence](../../reviews/2026-11-07-local-ide-acceptance.md). Fixed Compose forwarding, read-only MCP annotations, ambiguous publication metadata and the populated-server verifier. Native capture remains outside the qualified client versions; no capture safeguards were bypassed. The ChatGPT desktop UI was inaccessible to computer automation, so Codex results are from its bundled CLI, not an automated desktop UI run.
