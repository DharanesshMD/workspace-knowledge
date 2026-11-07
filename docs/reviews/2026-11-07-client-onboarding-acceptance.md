# Repeatable client onboarding acceptance

Follow-up to the [local IDE trial](2026-11-07-local-ide-acceptance.md), whose verified baseline was committed as `ca6c6d8`. This qualifies source-installed MCP onboarding on the local macOS arm64 host, using the real bundled Codex configuration CLI and the installed Antigravity configuration path. The previous report contains the model-driven IDE workflow evidence; this run exercises installation and credential/configuration lifecycle through actual native proxy processes.

## Delivered behavior

- The source installer installs five release binaries plus `wk-connect`; Python 3.11+ is checked before building.
- `connect` validates the actual authenticated MCP connection before replacing credentials. Token values are absent from command arguments, IDE settings and normal output; managed token/CA files use mode 0600 in a mode-0700 directory. These are private plaintext files, not Keychain storage.
- The IDE runs `wk-mcp proxy --token-file` directly. Bounded native reads reject symlinks, nonregular files, public permissions and malformed/oversized tokens; an explicit file takes precedence over an inherited `WK_TOKEN`.
- Complete Codex TOML policies are checked, including nested per-tool approval fields the CLI's `mcp get` projection omits. CLI updates are staged privately and refused if they discard unrelated settings. Changed live settings during staging are preserved; rollback only restores a config written by this invocation that still matches its committed bytes.
- Each explicit replacement keeps a private displaced-config snapshot. Reconnect, check, owned-entry disconnect, command removal and source reinstall are repeatable. Unrelated entries/files are preserved. Capture and OS certificate trust are not enabled or changed.

## Live acceptance

`python3 scripts/client-onboarding-acceptance.py --output-dir .tooling/onboarding-acceptance` passed all 12 checks against the existing local HTTPS `wk-stack` deployment. The run used a fresh HOME and install prefix containing spaces, invoked the installer from outside the checkout, and created one disposable backend-scoped developer.

| Check | Observed result |
|---|---|
| Actual source installation | Six working commands installed into the fresh prefix |
| New developer | Actual server-issued personal token authenticated |
| Invalid CA | Refused before IDE settings changed |
| Codex and Antigravity setup | Other settings/servers retained; direct installed native proxy; no embedded token |
| Removed original CA handoff | Both clients still worked using their private managed copies |
| Cross-client sharing and retrieval | Exact synthetic card retrieved; backend scope did not reveal frontend work |
| Server outage and restart | Both checks failed during outage and recovered afterwards |
| Token revocation | Old credential refused; replacement restored both clients |
| Token expiry | Only the disposable token's expiry was forced; replacement restored both clients |
| Disconnect | Two invocations succeeded; active managed credentials removed, other settings retained |
| Uninstall and reinstall | Unrelated file retained; all six same-source command hashes restored |
| Cleanup | Synthetic card withdrawn and disposable developer deleted, both HTTP 200 |

Local receipt: `.tooling/onboarding-acceptance/wk-clean-user-gnbzpgx0/results.json`. The withdrawn synthetic card is `886c7f7d-bf6f-468e-aee9-6b2ab1721bd6`; the deleted test developer was `onboard-f71571f8ff`. Private test directories and original configuration backups remain local and are not committed.

After acceptance, the actual user installation in `~/.local/bin` was updated. Both `workspace-knowledge-local` IDE entries were migrated from private trial launchers to the installed native proxy, preserving backups. Fresh `wk-connect check --client CLIENT --name workspace-knowledge-local` calls discovered all 12 tools for Codex (Ravi) and Antigravity (Anika). The application remained healthy. New client sessions or an MCP connection refresh are needed to load the new process configuration.

## Regression verification

| Check | Result |
|---|---|
| Rust workspace including PostgreSQL/pgvector tests, `--include-ignored` | 801 passed, zero failed or ignored |
| Python discovery | 58 run: 57 passed, one explicit Gitleaks-executable-dependent skip |
| New onboarding tests within Python discovery | 11 passed, including the real Codex CLI policy and concurrent-edit regressions |
| Strict workspace/all-target Clippy | Passed |
| Formatting and whitespace | Passed |
| Actual release source install | Passed in fresh prefix and normal user prefix |
| Focused independent review | Policy preservation, replacement backups and concurrent-edit findings fixed; no remaining actionable findings |

No web code changed in this step; the baseline's 77 passing browser tests were not repeated. Rust dependency versions were unchanged; `libc`, already in the lockfile, became a direct Unix MCP dependency. The Rust run still emits the existing duplicate `schema` example-output warning.

## Remaining boundary

A fresh HOME is not a new OS account, Keychain identity, clean physical machine or signed/notarized installer. Linux helper behavior and native Windows token-file onboarding are not qualified by this macOS run. This is a local source-installed evaluation workflow; it does not close public-provider, exact-final-commit remote CI, security review, signed distribution or real-team production pilot gates. Native capture remains experimental and disabled for these installed versions. Public release readiness remains governed by the [readiness ledger](../operations/production-readiness.md).
