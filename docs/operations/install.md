# Install and deploy

Choose one path: try the [synthetic demo](../../README.md#quickstart), join an existing team server, or operate a server. Developers joining a team do not need a local PostgreSQL database, Node.js or Docker. Installation does not approve capture.

This is a pre-release source build. The latest published source-only prerelease is `v0.1.0-alpha.3`; its tag predates current `main`. The intended production-support target is a Linux Docker server with macOS developer machines, but production acceptance is still open. Linux x64/arm64, Windows x64 and macOS arm64/x64 are build targets, not blanket fresh-install/service-startup certifications. Record exact OS, architecture, toolchain and client versions before claiming support. See the [production readiness ledger](production-readiness.md).

## Install the tools

Install Rust through rustup; the checkout pins the toolchain in `rust-toolchain.toml`. A C build toolchain is required. The macOS/Linux source installer also requires Python 3.11 or newer for `wk-connect` and checks that prerequisite before building. macOS needs Xcode Command Line Tools; Windows needs MSVC build tools. On Debian/Ubuntu, collector builds need `build-essential pkg-config libdbus-1-dev`. Persistent capture also requires a working user credential store: macOS Keychain, Windows Credential Store, or Linux Secret Service/DBus. Missing secure storage fails closed.

From a cloned checkout on macOS/Linux:

```sh
./deploy/scripts/install-source.sh
export PATH="$HOME/.local/bin:$PATH"
```

The script works from another directory when invoked by its absolute path. It installs `wk`, `wk-agent`, `wk-env`, `wk-server` and `wk-mcp` into `~/.local/bin`. Set `WK_PREFIX` to use another user-owned prefix. The PATH command applies to the current shell; add the directory to your shell configuration if you want it available in future sessions. Never run the collector installer as root.

**macOS and Gatekeeper.** Binaries you build with this script are signed ad hoc by the linker and are not quarantined (checked on macOS with `xattr` and `codesign -dv`), so macOS shows no unsigned-developer prompt. Only binaries downloaded from a browser or chat get quarantined; no signed or notarized binary is published yet, so prefer the source install above. The first run of a rebuilt `wk-agent` can still ask for Keychain access to its existing items: allow only the expected Workspace Knowledge items. A signed, notarized release needs an Apple Developer identity the project does not have yet ([readiness ledger](production-readiness.md)).

On Windows, run:

```powershell
cargo build --locked --release -p wk-cli -p wk-agent -p wk-environments -p wk-server -p wk-mcp
```

Copy `wk.exe`, `wk-agent.exe`, `wk-env.exe`, `wk-server.exe` and `wk-mcp.exe` from `target/release` to a user-owned directory on PATH. This does not register startup or install client hooks. Node.js 22/npm is needed only to build the web console; server Docker builds handle that in the build image.

## Join a team server

For Codex or Antigravity MCP access with a personal token, use the [repeatable client onboarding](client-onboarding.md) commands. `wk-connect` validates the connection and configures the installed proxy without a machine-specific launcher or tokens in IDE settings.

Get the portal URL, product name and permitted services from your administrator. Sign in to that portal through the team's OIDC provider. My context and Team context are available without installing a collector; installation adds local capture and synchronization.

To connect capture you do not need a model. The example below uses placeholders: replace the product path, product, service and server URL with your reviewed values. Without `--inference` the collector builds drafts from rules; to use a local model instead, run one and add `--model MODEL --inference URL` with its loopback OpenAI-compatible endpoint (see [setup without a local model](../adapters/setup.md#set-up-without-a-local-model)). Private state must stay outside the approved product folder.

```sh
wk --state-dir "$HOME/.local/share/workspace-knowledge/consent" workspace approve /absolute/work/product --product PRODUCT --service backend --yes
```

The response contains `binding_file`, the exact saved approval JSON path. Copy that value into `--binding` below; there is no need to calculate a filename or find a hash.

```sh
wk-agent --state-dir "$HOME/.local/share/workspace-knowledge/collector" device connect --binding /absolute/path/from/binding_file.json --service backend --server https://knowledge.internal --yes
wk-agent --state-dir "$HOME/.local/share/workspace-knowledge/collector" serve
```

If your server and identity provider use a private certificate authority, add `--ca-file /path/to/private-ca.pem` to `device connect`; the collector then trusts only that bundle for them. See [Keycloak setup: native collector](oidc-keycloak.md#native-collector).

For an AI agent's MCP access, run `wk-mcp login --server https://knowledge.internal` once (add `--ca-file` for a private CA); the stdio bridge then renews its access token silently from a refresh token kept in the OS credential store. See the [quick start](quick-start-keycloak.md#3-sign-in-once-and-connect-an-agent).

`device connect` opens the system browser for upstream public-client OIDC/PKCE sign-in. The token and installation signing key stay in the native OS credential store. Local state stores public configuration, approval receipts and credential expiry. Allow only expected Workspace Knowledge credential-store prompts. Test foreground operation before enabling startup.

In another terminal, run `wk-agent --state-dir "$HOME/.local/share/workspace-knowledge/collector" doctor`. Then follow the [adapter instructions](../adapters/setup.md) for your exact client/version. Connecting a device alone does not install hooks or establish capture coverage. Each collector state binds one chosen service; multiple services require explicit attribution.

Owner summary synchronization and activity sharing default off. If wanted, grant the matching hosted setting under Devices and explicitly enable the corresponding local permission; both are required. Owner sync uses `owner-sync enable --yes`. On current `main`, new collectors enable publication without a reviewer for eligible assistant-written cards. Set `WK_AUTO_PUBLISH_PRODUCTS=none` on the server or connect with `--no-auto-publish` to require review; existing collectors keep their prior choice. Rule drafts and hook captures still require review. A changed local scope, destination, model or summarizer mode requires a new explicit local connection/consent.

### Renewal and device controls

Run `wk-agent --state-dir PRIVATE_STATE device login` after upstream expiry. Automatic refresh-token storage is absent. `device status` reads safe local state; `device poll` retrieves typed controls and acknowledges them. `device check-native-policy` checks current signed policy authorization; success does not certify capture coverage.

The hosted Devices page lists only the signed-in developer's installations, without raw messages, local paths or credentials. Reports older than 90 seconds are stale; reports expire after at most 7 days. Server pause blocks uploads immediately; local pause remains pending until received by the collector. Remote resume preserves local pause. Revocation denies further server uploads and cannot reactivate the installation key; an offline collector may continue local capture until it observes revocation.

A collector token is bound to one installation through expiry plus 120 seconds. Replacing a revoked installation requires a fresh upstream sign-in with a distinct access token, for the collector audience only, with at most 12 hours remaining lifetime. At most 1,000 unexpired credential hashes per owner are retained; token text is not stored in that binding table. Transport outages retain held data under its existing TTL; authorization denial clears native held records.

Legacy `init`, environment-token replacement and `device connect --test-token` are for literal-loopback synthetic development only. They are not hosted sign-in instructions. An acknowledged pause prevents further drafting by `process` and discards pending results, although a request already sent to a local model cannot be recalled.

### Optional startup

Use the same private collector state as your foreground test. The provided Linux/macOS templates assume `~/.local/share/workspace-knowledge`; update their state argument to your actual collector directory before enabling them.

- Linux: copy `deploy/systemd/wk-agent.service` to `~/.config/systemd/user/`, inspect its executable/state paths, then run `systemctl --user daemon-reload` and `systemctl --user enable --now wk-agent`.
- macOS: replace `REPLACE_HOME` with your absolute home path, inspect executable/state paths, save the plist in `~/Library/LaunchAgents/`, and run `launchctl bootstrap gui/$(id -u) PATH_TO_PLIST`.
- Windows: run `deploy/windows/install-startup.ps1 -AgentPath ABS_EXE -StateDirectory ABS_PRIVATE_STATE` in the developer's user session.

Never run capture as root/SYSTEM. Stay in foreground mode if startup cannot access the user credential-store session. Stable signed packaging and native startup certification remain release gates.

## Host a team server

The Docker build includes the portal and server, so operators do not need to build Rust and Node locally. You need a Linux Docker/Compose host, PostgreSQL storage, an OIDC provider and a same-origin HTTPS reverse proxy. macOS/Windows Docker hosts use a Linux VM. The Compose file is a deployment template, not a certified appliance.

Configure secrets through your deployment secret manager. The required variables in `deploy/compose/compose.yaml` are:

| Variable | Purpose |
|---|---|
| `WK_POSTGRES_PASSWORD` | Protected database owner password |
| `WK_DATABASE_URL` | Runtime connection using the restricted `wk_app` role, never `wk_owner` |
| `WK_OIDC_ISSUER`, `WK_OIDC_AUDIENCE`, `WK_OIDC_JWKS_URL` | Fixed upstream HTTPS issuer, API audience and verification keys |
| `WK_OIDC_CA_FILE` | Optional PEM bundle for an identity provider behind a private CA; replaces public roots for provider requests only. Collectors take the bundle with `device connect --ca-file`. See [Keycloak setup](oidc-keycloak.md) |
| `WK_AUTO_PUBLISH_PRODUCTS` | Optional. Unset or empty (the default): every product lets a developer's collector publish their own assistant-written cards without a reviewer, if that collector has it on. `none` turns it off server-wide; a comma-separated list of product IDs enables only those. Any other value stops startup. See [publishing without review](../adapters/setup.md#publishing-without-review-on-by-default-for-new-setups). |
| `WK_MCP_RESOURCE`, `WK_MCP_CLIENT_ID` | Optional, for remote MCP and `wk-mcp login`: the server's public HTTPS origin with no trailing slash (it must equal `WK_OIDC_AUDIENCE`) and the public sign-in client. See the [Keycloak quick start](quick-start-keycloak.md) and [remote MCP](../architecture/remote-mcp.md) |
| `WK_PERSONAL_TOKENS`, `WK_PERSONAL_TOKEN_MAX_DAYS` | Optional. `WK_PERSONAL_TOKENS=1` lets each developer create revocable personal tokens in the portal's Connect page that authenticate to one product's hosted MCP address only (never the REST API), so an agent can connect without a browser sign-in. Off by default; any other value stops startup. The lifetime cap is 1 to 365 days, default 90. Existing databases need the `GRANT` for `personal_tokens` in `deploy/scripts/application-role.sql` re-applied. See the [threat model](../security/threat-model.md) |
| `WK_BROWSER_CLIENT_ID` | Registered portal authorization-code client |
| `WK_BROWSER_CLIENT_SECRET` | Optional secret for a confidential browser client; omit for a public client |
| `WK_CONSOLE_ORIGIN` | Exact public HTTPS portal origin |
| `WK_COLLECTOR_CLIENT_ID`, `WK_COLLECTOR_AUDIENCE` | Separate native public client and collector-only audience |

Register the browser redirect as `WK_CONSOLE_ORIGIN/auth/callback`. Register the native collector with literal `http://127.0.0.1:<ephemeral-port>/callback` support. Collector tokens must have exactly the collector audience, distinct from the API audience. The app does not create an OAuth issuer or infer memberships from provider claims. See [identity details](../architecture/backend.md#hosted-browser-identity). Keycloak is the one provider qualified so far; see its [setup and audience rules](oidc-keycloak.md), the prefilled realm and the [Keycloak quick start](quick-start-keycloak.md).

Build and start only the database first:

```sh
docker compose -f deploy/compose/compose.yaml build app
docker compose -f deploy/compose/compose.yaml up -d postgres
```

Before starting the app, use a separately protected operator database connection to run `wk-server migrate`, create the `wk_app` login role through your secret-management process, apply `deploy/scripts/application-role.sql`, and provision explicit issuer/subject mappings and memberships with `wk-server provision` JSON stdin. Operators can run these CLI commands in a one-off app container on the Compose network or from the installed tools. Do not pass owner credentials to the long-running app. The [backend guide](../architecture/backend.md) and [administration guide](hosted-portal.md#administrative-boundaries) describe those boundaries.

After provisioning:

```sh
docker compose -f deploy/compose/compose.yaml up -d app
```

Place the HTTPS reverse proxy in front of host-loopback port 8080. Preserve Origin headers and cookies; do not log callback queries, cookies or Authorization headers. Configure proxy deadlines, request-size/rate limits and backup retention. Run the separately authorized retention worker and rehearse deletion-safe encrypted restore using the [hosted operations](hosted-portal.md), [backup](backup.md) and [restore](restore.md) guides. On current `main`, `/health/ready` checks runtime database access and schema migrations 1–21; it does not certify capture, identity-provider health or backups. After migrating an older installation, reapply `deploy/scripts/application-role.sql` with the operator connection so the restricted runtime role can use the new tables and functions.

For a source-hosted server, build the console with `cd web && npm ci && npm run build`, then set `WK_WEB_DIR` to the absolute `web/dist` directory when starting `wk-server --production`. Assets are served from that directory; they are not embedded in the executable.

Production provider/HTTPS acceptance, target-platform startup, expanded privacy/client qualification and workload-specific capacity remain release gates. The prior bounded local five-developer pilot does not certify a production server's size or throughput.
