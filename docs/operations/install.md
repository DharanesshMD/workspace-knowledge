# Install from source and deploy

Pre-release certification status: Linux x64/arm64, Windows x64, macOS arm64/x64 are source-build targets. No fresh-install/service-startup certification is asserted for any exact OS version yet. Record OS, architecture, Rust, browser, database and client versions in release evidence before claiming support.

Install the pinned Rust toolchain in `rust-toolchain.toml`, a C toolchain and Node 22/npm for the console. On Debian/Ubuntu install `build-essential pkg-config libdbus-1-dev` for native keyring support. Windows needs MSVC build tools; macOS needs Xcode Command Line Tools. A working user credential store/session is required for persistent capture; absence fails closed, not to a plaintext key.

From repository root, `deploy/scripts/install-source.sh` installs `wk`, `wk-agent`, `wk-env`, and `wk-server` in `~/.local/bin` (or `WK_PREFIX/bin`). On Windows use `cargo build --locked --release --workspace` and copy those `.exe` binaries from `target/release` to a user-owned PATH directory. Build console with `cd web && npm ci && npm run build` before server release builds so its assets can be bundled. No installation command approves capture implicitly.

Enroll with `wk workspace approve` according to CLI help; then initialize the agent using its local approval JSON and explicit target service:
```
wk-agent --state-dir /absolute/private-state device connect --binding /absolute/approval.json --service backend --server https://knowledge.internal --model MODEL --inference http://127.0.0.1:11434/v1/chat/completions --yes
wk-agent --state-dir /absolute/private-state serve
```
Hosted `device connect` opens the system browser for upstream public-client OIDC/PKCE sign-in. The server needs `WK_COLLECTOR_AUDIENCE` distinct from `WK_OIDC_AUDIENCE` and `WK_COLLECTOR_CLIENT_ID`; register that native public client with literal `http://127.0.0.1:<ephemeral-port>/callback` redirect support and issue an access token for the collector audience only. A token listing multiple audiences is rejected. Browser sign-in remains a separate registered client. The application creates no OAuth issuer. Provider-specific compatibility remains a deployment acceptance gate.

The upstream access token and installation signing key stay in the native OS credential store. Local state holds public configuration/approval receipts and credential expiry only. Run `wk-agent --state-dir /absolute/private-state device login` after upstream expiry; automatic refresh-token storage is intentionally absent. `device status` reads safe local registration/settings state, and `device poll` retrieves typed controls and acknowledges them. A new local scope/destination/inference configuration needs a new explicit local connection; the hosted browser cannot approve it. Installation/source-build support is not native keyring or provider certification.

Owner synchronization and activity permissions default off for new hosted devices. Enable each only after reviewing both local consent and hosted settings; effective sharing requires both. Remote pause blocks new uploads immediately, while local capture pause is pending until the collector receives it. Remote resume preserves local pause. Reported connection health never certifies hook installation, complete capture or publication.

Legacy `init` and environment-token replacement are restricted to literal-loopback development. `device connect --test-token` likewise accepts a synthetic `WK_ACCESS_TOKEN` only for loopback; never use that fixture option for deployment. Foreground `serve` binds authenticated loopback. Test foreground mode and credential-store access before registering startup.

For Linux copy `deploy/systemd/wk-agent.service` to `~/.config/systemd/user/` and run `systemctl --user daemon-reload` then `systemctl --user enable --now wk-agent`. The template assumes state in `~/.local/share/workspace-knowledge`. For macOS replace `REPLACE_HOME` with the absolute home path in the plist, save it in `~/Library/LaunchAgents/`, and use `launchctl bootstrap gui/$(id -u) PATH`. For Windows invoke `install-startup.ps1 -AgentPath ABS_EXE -StateDirectory ABS_PRIVATE_STATE` in the developer's user session. Never install capture as root/SYSTEM. Use foreground mode if startup lacks the required credential-store session.

For Linux on-prem deployment, build with `docker compose -f deploy/compose/compose.yaml build`. Configure the required database/OIDC/origin variables through your secret-management environment. Start PostgreSQL first. Use a separate migration connection to run `wk-server migrate`, provision scoped identities using `wk-server provision` JSON stdin, and use an independently restricted database role for the app. Start `app` only after migration/provisioning. A TLS reverse proxy must forward to loopback 8080 and enforce request size/rate limits; no reverse-proxy credentials or certificates are fabricated here. macOS/Windows Docker hosts need a Linux VM. The compose example is an integration/deployment template, not a certified hardened appliance.

### Hosted Devices screen

Signed-in developers can open Devices within their selected product. The screen lists only their installations and contains no raw conversation messages, local paths or credentials. Refresh device reports to see a newly acknowledged revision. Reports older than90seconds are labelled stale; reports expire logically after7days. Collector declarations, connection and version do not certify capture coverage.

Save requested settings uses the current revision. Server pause blocks uploads immediately; local capture pause remains pending until the collector receives it. Owner summary sync and activity require local consent as well as hosted permission. Reload latest settings discards unsaved requests after a conflict. Revoke installation requires confirmation, denies future server uploads and cannot reactivate that installation key; an offline collector may continue local capture until it observes the revocation. Folder, inference destination and credentials remain locally approved.


### Collector credential renewal and diagnostics

A verified collector credential is bound to one installation through its token expiry plus two minutes. A revoked installation cannot reuse that credential to enroll a replacement key. Reconnect through a fresh upstream sign-in; configure the provider to issue a distinct access token per sign-in, with at most twelve hours remaining lifetime. Renewed tokens may bind to the existing installation on its next signed request. The separate collector audience remains mandatory; collector tokens cannot authorize ordinary API or MCP calls. Up to1000 unexpired credential hashes per owner are retained; token text is never stored in that binding table.

`wk-agent --state-dir PRIVATE_STATE device check-native-policy` checks current native policy authorization through the signed API. Success does not certify capture coverage. Transport outages retain held data under its existing TTL; actual authorization denial clears native held records. An acknowledged pause prevents further inference attempts and discards an already pending result, although an HTTP request already sent to the model cannot be recalled.
