# Install from source and deploy

Pre-release certification status: Linux x64/arm64, Windows x64, macOS arm64/x64 are source-build targets. No fresh-install/service-startup certification is asserted for any exact OS version yet. Record OS, architecture, Rust, browser, database and client versions in release evidence before claiming support.

Install the pinned Rust toolchain in `rust-toolchain.toml`, a C toolchain and Node 22/npm for the console. On Debian/Ubuntu install `build-essential pkg-config libdbus-1-dev` for native keyring support. Windows needs MSVC build tools; macOS needs Xcode Command Line Tools. A working user credential store/session is required for persistent capture; absence fails closed, not to a plaintext key.

From repository root, `deploy/scripts/install-source.sh` installs `wk`, `wk-agent`, `wk-env`, and `wk-server` in `~/.local/bin` (or `WK_PREFIX/bin`). On Windows use `cargo build --locked --release --workspace` and copy those `.exe` binaries from `target/release` to a user-owned PATH directory. Build console with `cd web && npm ci && npm run build` before server release builds so its assets can be bundled. No installation command approves capture implicitly.

Enroll with `wk workspace approve` according to CLI help; then initialize the agent using its local approval JSON and explicit target service:
```
wk-agent --state-dir /absolute/private-state init --binding /absolute/approval.json --service backend --server https://knowledge.internal --model MODEL --inference http://127.0.0.1:11434/v1/chat/completions --yes
wk-agent --state-dir /absolute/private-state serve
```
`WK_ACCESS_TOKEN` is required for initialization and saved via native keyring. Supply it from a trusted credential launcher; never put it in shell history or a service file. Foreground `serve` binds authenticated loopback. Test foreground mode and credential-store access before registering startup.

For Linux copy `deploy/systemd/wk-agent.service` to `~/.config/systemd/user/` and run `systemctl --user daemon-reload` then `systemctl --user enable --now wk-agent`. The template assumes state in `~/.local/share/workspace-knowledge`. For macOS replace `REPLACE_HOME` with the absolute home path in the plist, save it in `~/Library/LaunchAgents/`, and use `launchctl bootstrap gui/$(id -u) PATH`. For Windows invoke `install-startup.ps1 -AgentPath ABS_EXE -StateDirectory ABS_PRIVATE_STATE` in the developer's user session. Never install capture as root/SYSTEM. Use foreground mode if startup lacks the required credential-store session.

For Linux on-prem deployment, build with `docker compose -f deploy/compose/compose.yaml build`. Configure the required database/OIDC/origin variables through your secret-management environment. Start PostgreSQL first. Use a separate migration connection to run `wk-server migrate`, provision scoped identities using `wk-server provision` JSON stdin, and use an independently restricted database role for the app. Start `app` only after migration/provisioning. A TLS reverse proxy must forward to loopback 8080 and enforce request size/rate limits; no reverse-proxy credentials or certificates are fabricated here. macOS/Windows Docker hosts need a Linux VM. The compose example is an integration/deployment template, not a certified hardened appliance.
