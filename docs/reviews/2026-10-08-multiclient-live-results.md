# Codex, Claude Code and Antigravity live tests — 2026-10-08

Implementation: `feb0978` (no runtime changes for this test). An isolated synthetic workspace, loopback API, disposable PostgreSQL database and read-only developer B identity were used. A published synthetic card contained a random marker that was not included in any client prompt. These are bounded interoperability observations, not production certification.

| Client / surface | Live result | Capture result |
| --- | --- | --- |
| Codex CLI 0.162.0-alpha.2 | Passed actual `knowledge.search` and `knowledge.get`; returned AMBER-AA2260 and required idempotency keys with a maximum of three retry attempts. | Native adapter remains unimplemented. Codex Desktop was not separately tested. |
| Antigravity desktop 2.19.1, Gemini 3.8 Flash Medium | Passed the same two actual MCP calls and returned the correct marker and requirements. UI reported 24 seconds. | Native adapter remains unimplemented. Antigravity IDE 2.5.5 was installed but not tested. |
| Claude Code CLI 2.1.15 | Two bounded attempts timed out without output. | No events from those attempts. |
| Claude Code CLI 2.1.293, binary bundled with Claude Desktop | MCP connected and nine tool definitions appeared during the proxy attempt. Model requests failed against the pre-existing localhost:4141 proxy. A process-only direct-provider retry reached the API but returned `Credit balance is too low` (HTTP 400). No successful model-driven retrieval. | Two actual UserPromptSubmit callbacks captured two prompts. Final collector counters: three events, two prompts, zero responses, zero gaps. No successful response or full-turn capture claim. |

## Setup findings

Codex noninteractive execution initially rejected MCP calls because tool approval was required while its approval policy was `never`. A per-tool `approval_mode` setting was unrecognized by this installed build. The successful run used the supported `--approve-for-me` review route, with only search/get enabled; no approval or sandbox bypass flag was used. Search returns card references, so fetching `knowledge.get` is necessary to read decisions and the marker.

Antigravity discovered the nine tools, but its settings UI displayed `0 tools enabled`. Attempts to enable search/get did not visibly change that count; nevertheless, the actual chat successfully invoked both tools. This run therefore establishes functional retrieval, not reliable settings-display behavior. The chat's preliminary file reads were MCP tool-definition JSON files shown by its own discovery UI, not product source files. Workspace configuration alone was not established as sufficient: a temporary global entry was also added, preserving all existing entries, then removed after the test. Configuration followed the [official MCP documentation](https://antigravity.google/docs/mcp).

Claude's direct retry removed proxy/auth environment overrides only for that subprocess and used the existing login; global authentication settings were not changed. Its final initialization listed zero tools despite a connected server, whereas the earlier attempt listed nine. Billing prevented establishing successful tool invocation in the direct retry. No purchase, account change or credit top-up was attempted. Claude Desktop general chat/Enterprise Compliance capture was not tested.

## Evidence and limits

Content-limited synthetic evidence is retained locally under `.superpowers/sdd/2026-10-08-multiclient-live/`: `codex-retrieval.json`, `antigravity-observation.json`, `claude-result-summary.json` and `final-doctor.json`. The Antigravity record is a structured summary of the desktop accessibility observations; it is not an exported vendor transcript. The complete test card was seeded through the API, not automatically captured from Codex or Antigravity. The earlier Cursor capture-to-reviewed-publication test is documented separately in [Cursor live acceptance](2026-10-08-cursor-live-acceptance.md).

This test does not establish native capture for Codex/Antigravity, complete Claude capture, unattended operation, five-developer load behavior, or cross-version compatibility. Those adapter and certification gaps remain unchanged.

## Cleanup

Temporary Antigravity global/local MCP entries were removed, existing global entries preserved, and synthetic Claude hooks disabled. Owned API/collector processes and the disposable database container were stopped. Synthetic Keychain items, private collector state and temporary credentials were removed. Synthetic project folders/chat and the stopped database's synthetic records remain for traceability. No real project hooks or conversation histories were modified.
