# Repeatable client onboarding

Goal: replace the private per-machine launchers from the local IDE trial with installed tools, preserving existing IDE settings and testing the complete lifecycle in a fresh user configuration.

Approved scope: the user's instruction to proceed with the installer and clean-user acceptance step. Keep the first release focused on MCP; native capture remains experimental.

Implementation choices: ship `wk-connect`, a Python-standard-library helper, beside the existing five binaries. It configures Codex through its own CLI and Antigravity through its JSON configuration. Add `wk-mcp proxy --token-file` so clients execute the native proxy directly without a launcher or token environment variable. Validate the connection before updating existing configuration, keep private backups, and provide check/disconnect commands. Do not change global certificate trust or approval policies.

- [x] Commit the previously verified local IDE fixes as the baseline (`ca6c6d8`).
- [x] Add a failing regression for private token-file loading; implement bounded, no-symlink, private-file reads and CLI selection without changing environment-token compatibility.
- [x] Add failing onboarding lifecycle tests; implement connection validation, private credential/CA state, configuration preservation, repeatable updates, check and owned-entry disconnect.
- [x] Install/remove `wk-connect` through the source installer and extend its existing lifecycle tests.
- [x] Run actual installed tools and the real Codex configuration CLI under a fresh HOME/prefix against the local HTTPS server. Exercise retrieval/sharing, restart, credential replacement, revocation, outage and removal without modifying normal user configuration.
- [x] Run relevant full suites, obtain a focused review, document exact results and limitations, and commit the completed change.

Outcome: [12-check acceptance and final verification](../../reviews/2026-11-07-client-onboarding-acceptance.md). The normal local Codex and Antigravity entries were migrated after the isolated acceptance passed. Python 3.11's standard TOML parser preserves complete Codex policies; staging guards refuse unrelated CLI changes or concurrent edits rather than overwriting them.

Acceptance limits: a fresh HOME is not a fresh OS account, distinct Keychain identity, signed installer, or clean-machine certification. Existing billing and external pilot requirements remain outside this local setup step.
