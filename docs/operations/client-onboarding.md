# Connect Codex or Antigravity

This is the repeatable, source-installed MCP setup for the first evaluation release. It retrieves team knowledge and shares summaries on request. Automatic conversation capture remains experimental and is not enabled by these commands.

## Prerequisites

- A provisioned Workspace Knowledge server, its product MCP URL (`https://HOST/mcp/PRODUCT`), and a personal MCP token issued by your administrator or created in the portal's Connect page. The operator must enable personal tokens.
- macOS or Linux, Python 3.11 or newer for `wk-connect`, and the relevant IDE. The helper uses Python's standard TOML parser to preserve complete Codex tool policies. Codex setup also needs its CLI; the macOS ChatGPT-bundled CLI is detected when it is not on PATH, or can be selected with `--codex PATH`.
- The installed `wk-mcp` and `wk-connect` tools. From a source checkout run `./deploy/scripts/install-source.sh` and add the printed installation directory to PATH. The installer includes all five native commands and the onboarding helper; it enables no capture, hooks or startup services.

The live qualification so far is macOS arm64 with a Linux Docker server. A fresh HOME test does not certify another OS account, Keychain session or physical machine. This helper is POSIX-only; existing Windows environment-token configuration remains separate.

## Connect

Use your administrator's exact product URL:

```sh
wk-connect connect --client codex --url https://knowledge.example/mcp/PRODUCT
wk-connect connect --client antigravity --url https://knowledge.example/mcp/PRODUCT
```

Each command prompts for the token with input hidden. No token is placed in command-line arguments, project rules or IDE configuration. For automation, use `--token-stdin`; to copy a received private regular file, use `--token-file /absolute/private/token` (mode 0600). Do not paste tokens into shell command text.

For a server behind a private CA, add `--ca-file /absolute/path/ca.pem`. The bundle is validated and copied into managed user state. Neither the OS trust store nor certificate validation is changed. Setup checks the authenticated MCP connection before updating the IDE entry; invalid credentials, an invalid CA or an unreachable server leave the previous working credentials and settings in place.

Start a new Codex session or refresh Antigravity's MCP connections. The server entry is called `workspace-knowledge`; use `--name NAME` consistently on all commands for another name. Setup uses Codex's own configuration CLI and the current Antigravity path `~/.gemini/config/mcp_config.json`.

Unrelated settings and MCP servers are preserved. A different existing server with the same name is refused; choose another `--name`, or explicitly use `--replace` after reviewing the existing entry. Private configuration backups are kept under `~/.local/share/workspace-knowledge/clients/CLIENT/NAME/`.

Codex changes are staged through its native CLI and compared with the complete TOML configuration before being saved. If the CLI would discard an unrelated setting, or the live configuration changes during staging, setup refuses the update. Reconnect and disconnect also refuse manually changed tool policies. Each explicit replacement retains a separate private snapshot of the displaced configuration.

The IDE executes the installed native `wk-mcp proxy` directly, with a private token-file argument and, when needed, a managed CA-file argument. Each endpoint has its own credential directory. Changing the URL retains the previous endpoint's private files so a cached IDE command cannot send the new endpoint's token to the old server. Disconnect removes these retired credentials too. No generated launcher script or shell environment is needed to start the MCP server. Python is used for setup/check/removal, not for MCP traffic.

## Check and renew

```sh
wk-connect check --client codex
wk-connect check --client antigravity
```

The check opens a fresh proxy process and reports authenticated tool discovery without showing credentials. Ask your coding agent about teammates' recent work; it should call `knowledge.team_work`. Team-work `published` status is separate from implementation `lifecycle`.

After revocation or expiry, obtain a replacement token and repeat the same `connect` command. It validates the replacement before updating the private token file. Restart the IDE's MCP connection to ensure an already running proxy stops using its old in-memory credential. Server outages are reported as connection failures; retain your settings and retry `check` when service returns.

## Disconnect and uninstall

```sh
wk-connect disconnect --client codex
wk-connect disconnect --client antigravity
```

Disconnect removes only the managed entry, its active and retired endpoint token/CA files, and its connection manifest. Other servers are preserved. If you changed that entry manually, removal refuses to overwrite your changes. Backups are retained for manual recovery. Disconnecting locally does not revoke a server token; revoke it in the portal or through your administrator when retiring it.

The source checkout's `deploy/scripts/uninstall.sh` removes the six installed commands from `WK_PREFIX` (default `~/.local`) and preserves private state and unrelated files. Disconnect before removing the tools. Reinstalling the same source revision restores the commands without enabling capture or recreating a disconnected MCP entry.

For server administrators, `python3 scripts/client-onboarding-acceptance.py` runs the bounded local-demo lifecycle against `wk-stack`, using a new private HOME/prefix and a disposable developer. It briefly stops/restarts the app and forces expiry only on its newly issued test token. It removes its account and withdraws its synthetic card afterwards. Do not use it against a valued production deployment.
