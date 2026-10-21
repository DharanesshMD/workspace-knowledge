# Adoption friction: prioritized work

**Status: item 1 implemented and tested locally (see the [real-client results](../../reviews/2026-10-19-remote-mcp-signin-results.md)); item 2 (assistant-written default, quality comparison, rule-draft fixes) done, see the [comparison](../../reviews/2026-10-19-summary-quality-comparison.md) — its main finding is that partial cards cannot be published; item 5 (source-install path) done; items 3–4 proposed as separate sessions; signed binaries still need an Apple Developer identity.** Source: the "where it won't be smooth" assessment of 2026-10-19 for a small team self-hosting the alpha. Order is by developer pain removed per unit of effort, with items that need owner resources last. None of this changes the [production readiness](../../operations/production-readiness.md) gates.

## Priority order

| # | Problem | Why this rank | Blocked on owner? |
|---|---|---|---|
| 1 | Agents cannot "log in once" to the hosted MCP | Every developer hits it on day one; without it the shared server is hard to use. Testable locally with Keycloak and the real Claude Code, Cursor and VS Code installed here | No |
| 2 | Summaries are weak by default | The product's value is the quality of shared summaries. Cheapest improvement is steering users to the assistant-written flow that already exists | No |
| 3 | Capacity unmeasured; search p95 about 0.9 s at five developers | Database CPU is the measured limit, so tuning may be a cheap win; then a longer soak with more identities | No (real-team pilot still needs the owner) |
| 4 | Capture partial and version-pinned | Continuous, never "done"; make the support boundary explicit and detect drift instead of widening it | No |
| 5 | No signed macOS binaries | Mostly avoidable while distribution is source-only; real signing needs an Apple account | Yes |

## 1. Log in once to the hosted MCP

Today `wk-mcp` needs a hand-supplied short-lived `WK_ACCESS_TOKEN`, and native remote-MCP sign-in is untested.

- **1a. Quick-start and starter realm.** One page covering Keycloak + Compose + bridge, and an optional prefilled Keycloak realm (clients, audience mappers without the stock `account` audience) so a team does not hand-build it. Docs and fixtures only.
- **1b. `wk-mcp login`.** Authorization-code + PKCE with a loopback redirect (as `wk-agent device connect` already does), refresh token kept in the OS credential store, `wk-mcp` refreshing silently. Reuses the collector's login and private-CA code. Removes the manual-token step.
- **1c. Real-client remote OAuth.** Run `claude mcp add --transport http`, then Cursor and VS Code, against the local Keycloak stack. Expected hurdles to confirm, not assume: dynamic client registration, whether tokens carry the single required audience for the MCP resource, refresh behaviour. Record per-client, per-version results; fix confirmed defects only.
- **Done when:** a new developer connects Claude Code to the hosted server with one login and no copied token, verified end to end by the Keycloak acceptance harness.

## 2. Better default summaries

- Make the assistant-written flow (`wk-agent prompt` + `wk-agent draft`, `/wk-summarize`) the recommended default for clients that support it; keep rules mode as the zero-setup fallback.
- Measure with the existing history-evaluation script on synthetic or consented fixtures, comparing rules, assistant and local-model drafts; fix the worst rule-extraction failures found.
- Keep everything private and reviewed before sharing.
- **Done when:** the setup guide leads with the assistant flow and there is a recorded quality comparison.

## 3. Capacity and search performance

- Profile where database CPU goes under search/MCP load (the 2-core database hit its cap at five identities); look at query plans, indexes and per-request work before adding hardware.
- Re-run the pilot at 20 and 50 synthetic identities and for at least 30 minutes; publish the operating envelope for a small team only.
- **Done when:** search p95 is lower or the cap is explained, and the envelope is recorded. A real-team pilot remains an owner-scoped gate.

## 4. Capture support boundary

- One generated support table (client, version, OS, what is captured, what is held) that README, setup guide and completion matrix all link to.
- A scheduled check that flags new Claude Code and Cursor releases so unsupported versions are noticed rather than silently half-working.
- Codex and Antigravity stay experimental; Windows stays unsupported. Point everyone else at the universal fallback: explicit export and the assistant-written flow.
- **Done when:** unsupported versions fail with a clear message and the table is the single source of truth.

## 5. macOS distribution

- While releases are source-only, prefer a source install (Homebrew tap that builds from source, or `install-source.sh`). Locally built binaries are not quarantined, so no unsigned-binary warning.
- When the owner has an Apple Developer identity: protected signing and notarization workflow, clean-Mac install/upgrade/rollback test, then attach binaries.
- **Done when:** the install path has no Gatekeeper prompt, or signed and notarized artifacts pass a clean-machine check.

## Out of scope here

Hosted OIDC provider selection, the real-team pilot, independent security review and CI billing are the launch gates in the readiness ledger. The parked [Graphicle evaluation](future-graphicle-evaluation.md) stays parked.
