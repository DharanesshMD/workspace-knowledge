# Connect your coding agent

Workspace Knowledge works in Claude Code, Claude Desktop, Codex, Cursor and VS Code (GitHub Copilot). Two things have to happen for each: the agent must be able to **connect** (with your personal token), and it must be told, in standing instructions, **when to use the tools**, so it checks what teammates did before editing, answers "what did sam do last week?", and shares its work. Without the instructions an agent only uses the tools when you ask for them by name.

The files below come from one definition in the server. You can get them three ways, and they are always identical:

- the portal: **Connect**, then **Set up your coding agent** (and, for an admin, the message in the **Admin** tab, which also contains the token);
- the API: `GET /v1/products/{product}/agent-setup` (any member; no token in the reply);
- the demo script: `deploy/stack/make-demo-folders.sh`, which writes them into one folder per agent.

You need your product's MCP address (`https://HOST/mcp/PRODUCT`) and a personal token (from the portal's Connect page, or from the admin). Put the token in the environment variable `WK_TOKEN`; no file below contains it, except Claude Desktop's own settings file.

## What each agent needs

| Agent | Connection file | Standing instructions | Token reaches it via |
|---|---|---|---|
| Claude Code | `.mcp.json` | `CLAUDE.md` | `WK_TOKEN` in the shell that starts `claude` |
| Claude Desktop | `claude_desktop_config.json` entry running `wk-mcp proxy` | a Claude Project's instructions | the entry's `env` block |
| Codex | `.codex/config.toml` (or `~/.codex/config.toml`) | `AGENTS.md` | `WK_TOKEN` in the shell that starts `codex` |
| Cursor | `.cursor/mcp.json` | `.cursor/rules/workspace-knowledge.mdc` (or `AGENTS.md`) | `WK_TOKEN` in the environment Cursor starts with |
| VS Code (Copilot) | `.vscode/mcp.json` | `AGENTS.md` (or `.github/copilot-instructions.md`) | VS Code asks for it once and stores it securely |

### Claude Code

```sh
export WK_TOKEN=wkpt_...            # your token
cd your-project && claude           # with .mcp.json and CLAUDE.md saved in the project
```

Approve the project server when asked, then run `/mcp`. The same `.mcp.json` is safe to commit: it only names `${WK_TOKEN}`. For a private certificate authority start with `NODE_EXTRA_CA_CERTS=/path/to/ca.pem claude`. A one-command alternative stores the token in your Claude Code settings instead: `claude mcp add --transport http workspace-knowledge URL --header "Authorization: Bearer $WK_TOKEN"`.

### Claude Desktop

Claude Desktop's own custom connectors are called from Anthropic's servers, so they cannot reach a local or private-CA server, and they do not use personal tokens. Instead Claude Desktop starts a small local helper, `wk-mcp proxy`, which forwards every message to your product's MCP address with the token and trusts your private CA:

```json
{ "mcpServers": { "workspace-knowledge": {
    "command": "/absolute/path/to/wk-mcp",
    "args": ["proxy", "--url", "https://HOST/mcp/PRODUCT"],
    "env": { "WK_TOKEN": "wkpt_...", "WK_SERVER_CA_FILE": "/path/to/ca.pem" } } } }
```

Install `wk-mcp` (see the [install guide](install.md)), use `which wk-mcp` for the path, put this in `claude_desktop_config.json` (Claude menu, Settings, Developer, Edit Config), keep the file private (`chmod 600`: Claude Desktop starts the helper without your shell, so the token has to be in this file), and fully quit and reopen Claude Desktop. Drop `WK_SERVER_CA_FILE` for a public certificate. Claude Desktop has no project folders, so paste the standing instructions into a Claude Project's instructions. `wk-mcp proxy` needs no folder approval and no browser sign-in. (The older `wk-mcp` bridge with `wk-mcp login` still works for one signed-in developer per Mac user; it cannot use a personal token because it calls the REST API, which refuses them by design.)

### Codex

```sh
export WK_TOKEN=wkpt_...
codex mcp add workspace-knowledge --url https://HOST/mcp/PRODUCT --bearer-token-env-var WK_TOKEN
```

or save `.codex/config.toml` (Codex reads it for trusted projects):

```toml
[mcp_servers.workspace-knowledge]
url = "https://HOST/mcp/PRODUCT"
bearer_token_env_var = "WK_TOKEN"
```

and save `AGENTS.md` at the project root. The variable must be set in the shell that launches Codex. If your server uses a private CA, trust it in your operating system's certificate store.

### Cursor

Start Cursor from a shell where `WK_TOKEN` is exported (`open -a Cursor .`); an app opened from the Dock does not see your shell's variables. Save `.cursor/mcp.json` and the rule file, then check Settings, Tools & MCP shows the server's tools. If Cursor sends the literal text `${env:WK_TOKEN}` instead of the token (a bug some versions had with remote servers), put the header value in your user-level `~/.cursor/mcp.json` and keep that file private.

### VS Code (GitHub Copilot)

Save `.vscode/mcp.json` and `AGENTS.md`. Open the file and choose **Start** above the server (or run **MCP: List Servers**); VS Code asks for the token once, stores it securely and does not write it to the file. Use Copilot Chat in **Agent** mode; MCP tools are not available in plain chat.

## Check that it works

In any of them, ask: **"What did my teammates work on recently?"** The agent should call `knowledge.team_work` and tell you who did what, or say that nothing has been shared yet. Then try a named teammate, a directory (`backend/payments`) and a file.

## What the standing instructions say

Three things, in about 2 KB: check `knowledge.overlaps` before editing and tell you plainly when a teammate already did similar work; answer questions about teammates, files and modules with `knowledge.team_work`; and share with `knowledge.share_session`, including the files changed. **Sharing is opt-in by default**: the agent shares only when you ask. In the portal you can tick "share finished work automatically" to switch the instruction to sharing at the end of each task. The server also announces the same instructions when an agent connects, so a client that shows server instructions behaves correctly even without a file.

## What is and is not verified

Instructions steer a model; they do not force it. Only a hook would, and none is set up here. These are the facts as of the release that added this page, with the documentation each file follows (2026-10-26):

| Agent | Verified by the project's tests | Not verified (follows the documentation) |
|---|---|---|
| Claude Code | A real Claude Code connected with the generated `.mcp.json` and given four scripted tasks with the generated `CLAUDE.md`: it checked for teammates' work before editing and asked first when someone had already done it, answered a teammate question with `knowledge.team_work`, shared its change with the file list (sharing was switched on), and left a read-only question alone. | Behavior on other tasks and other models. |
| Claude Desktop | `wk-mcp proxy` end to end against a real server with a scripted client (the process Claude Desktop launches). | The Claude Desktop app itself. |
| Codex | The TOML parses and matches the documented fields. | Codex itself (not installed in the test environment). |
| Cursor | The JSON matches the documented fields. | Cursor itself, and `${env:}` in remote headers on your version. |
| VS Code | The JSON matches the documented fields. | VS Code and Copilot Chat themselves. |

Documentation followed: [Codex MCP](https://learn.chatgpt.com/docs/extend/mcp.md?surface=cli) and [AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md), [Cursor MCP](https://cursor.com/docs/mcp) and [rules](https://cursor.com/docs/context/rules), VS Code [MCP configuration](https://code.visualstudio.com/docs/copilot/reference/mcp-configuration) and [custom instructions](https://code.visualstudio.com/docs/copilot/customization/custom-instructions), [Claude Code MCP](https://code.claude.com/docs/en/mcp). Client formats change; the single definition to update is `crates/contracts/src/agent_setup.rs`.

## If something is off

| Symptom | Likely cause |
|---|---|
| The agent never calls the tools unless told | The standing-instructions file is missing from the project (or the agent ignores it). Ask it directly: "call knowledge.team_work". |
| "refused the token (HTTP 401)" | The token expired or was revoked, or this is not the token for this product. Ask your admin for a new one. |
| "denied access (HTTP 403)" | Your user no longer belongs to the product. |
| Cursor or Codex shows no server | `WK_TOKEN` was not set in the environment they were started from; quit, export it, start again. |
| Certificate errors | The server uses a private CA: see the notes for your agent above. |
