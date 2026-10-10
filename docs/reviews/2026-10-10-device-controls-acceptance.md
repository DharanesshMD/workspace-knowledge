# Device setup and controls: local acceptance

Date:2026-10-10. Local implementation and acceptance on branch `codex/cursor-native-capture`; no deployment, push or merge. Production provider selection remains pending.

## Implemented boundaries

Owner-only, product-scoped Devices UI supports versioned pause, owner-sync/activity permissions, manual report refresh and irreversible installation-key revocation. Requested controls, last acknowledged revision, local consent, report age and collector-declared capability are shown separately. New folders, inference destinations and credentials require local approval. Raw conversations, absolute paths and credential values do not enter device reports or this UI.

Upstream public-client OIDC/PKCE issues collector credentials for a separate audience. Exact-byte Ed25519 application proofs bind installation, hosted origin, route, timestamp, nonce, token and body. This is an application device binding, not OAuth DPoP. Ordinary API/MCP reject collector credentials. PostgreSQL locks device and membership through guarded writes and enforces current permissions, replay protection and owner-only report access. A newly connected collector defaults owner sync and activity off locally and remotely.

## Evidence

- Rust workspace:209 tests passed with all disposable PostgreSQL cases enabled;0 failures/ignored. Workspace clippy all targets with warnings denied, formatting and whitespace checks passed.
- Web:30 Playwright tests passed, plus TypeScript/Vite build. Device tests cover empty setup, stale/unknown reports, expected revisions, pause/revoke confirmation, stale-form discard after conflict reload, delayed product-response cancellation, keyboard controls and390px overflow. Initial missing-screen and conflict regressions were observed RED, then GREEN.
- Real acceptance: `scripts/portal-device-smoke.py --public-client` used actual Rust server, upstream synthetic OIDC HTTP endpoints, PostgreSQL and Chromium without API mocks. Native `wk-agent device connect --no-browser` completed PKCE, stored synthetic credentials/signing key in macOS Keychain, polled/report-acknowledged controls, preserved local pause under remote resume and reauthenticated after forced credential expiry. Synthetic OS entries were deleted on exit.
- Live hosted browser requested pause while collector polling was stopped: revision2 stayed pending with revision1 report; next real collector poll acknowledged2. Browser observed that acknowledgment, resumed, and revoked; the native collector's next poll was denied.
- Two upstream identities (Alice/backend, Bob/frontend/reviewer) could not see peer device metadata or owner summaries. Owner upload was denied before remote permission, accepted afterward within scope, and wider service uploads denied. Exact request replay, collector access to ordinary API/MCP, post-revocation uploads and re-registration of the same key were rejected. Separate ephemeral proof fixtures exercised failure paths without extracting native Keychain secrets.
- Local-only dev OIDC loader accepted actual literal-loopback provider keys; regression proved production constructors still reject HTTP and fixture loader rejects hostname/non-loopback/credentialed/query URLs. Missing loader was observed RED, then GREEN.
- Desktop/mobile live screenshots were inspected by the author: [desktop](device-controls/live-desktop.png), [mobile](device-controls/live-mobile.png).

## Limits and release gates

System-browser dispatch was not exercised in this acceptance: native sign-in used `--no-browser` and the harness followed the authorization redirect. The chosen production provider, HTTPS proxy deployment and Windows/Linux credential-store/browser support remain unverified. No refresh tokens are stored; upstream expiry requires `device login`. Offline capture remains fail-closed when current server policy cannot be checked; held local data retains its existing TTL. Capture declarations default unknown and do not upgrade prior native evidence. Codex/Antigravity version-specific limitations remain as documented in their capture reports.

Logical seven-day report expiry and thirty-day private-summary expiry are enforced; physical cleanup, deletion-ledger restore replay and five-developer capacity measurement belong to the remaining operations increment. Application operators/database/backup administrators remain trusted; this is not end-to-end encryption.

Known release security follow-up: `RUSTSEC-2023-0071` in `rsa0.9.10`, reached through runtime `openidconnect` in server and agent. Application OIDC paths perform public-key verification, with private RSA signing confined to synthetic test-provider fixtures; the dependency advisory remains unresolved. Do not treat this report as a clean security audit.

## Rulings made during implementation

- Ruling: continue approved device increment without renewed design permission — architecture already authorizes device authentication and typed controls — cost if wrong: reversible local implementation revision.
- Ruling: isolate collector audience and supplement upstream auth with existing Ed25519 request signatures — ordinary API/MCP cannot act as a revoked collector escape — cost: IdP must register a distinct collector audience/public client; this application binding requires security review and is not claimed as OAuth DPoP.
- Task1 Ruling: cap active owner installations at10 and retain at most1000 installation key tombstones, list up to20 with active devices first — revoked credentials cannot resurrect but normal replacements must remain possible — cost: operator intervention after1000 lifetime registrations; regression exposed the initial lifetime10 cap and was fixed before release. Unreleased migration8 helper refreshed only in disposable wk_test for current acceptance.
- Task2 Ruling: use upstream reauthentication when credentials expire rather than storing refresh tokens — provider selection is pending and refresh-rotation semantics are not yet verified — cost: collector requires device login after upstream expiry; no silent permanent credentials.
- Task2 Ruling: keep existing fail-closed capture while remote policy is unavailable — do not weaken native scope/membership gates to invent offline coverage — cost: offline capture may have gaps; existing retention and local held records remain bounded.
- Task2 Ruling: telemetry defaults every capture client/surface to unknown until the device has evidence — connection/registration cannot certify hook installation or capture — cost: capability checklist still needs explicit native verification; bounded native acceptance remains separately documented.
- Task2 Ruling: add signed activity-clear within the registered services — local activity opt-out must be able to clear its reports even when sharing permission is off — cost: clearing affects the owner's reports within those service scopes, not an individual device session.
- Task2 Ruling: expand replay/request window cap from120 to1200 signed requests per device/120seconds — existing native reconciliation and policy rechecks can exceed1request/second during normal work — cost: larger bounded nonce table; five-developer pilot must measure this10request/second ceiling. Burst regression observed RED at121, then GREEN after adjustment.
- Task3 Ruling: use explicit report refresh and a30second stale-age clock in the initial screen — keep unsaved controls from being discarded by background reload; conflict reload explicitly clears drafts — cost: newly acknowledged configuration is visible after Refresh until background polling is separately added.
- Task4 Ruling: add an explicitly named dev-only literal-loopback OIDC loader for collector acceptance — opaque fixture tokens cannot verify real upstream native sign-in; keep production constructors HTTPS-only and route roles separate — cost: additional fixture configuration surface, covered by production rejection tests.

## Independent review

Pending one fresh reviewer of the device increment and one verification pass for substantive findings.
