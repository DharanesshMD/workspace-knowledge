# Let an AI agent set up a server over SSH

An administrator gives an AI coding agent SSH access to a Linux host and the repository URL, and the agent installs a complete Workspace Knowledge stack: PostgreSQL, Keycloak, the server and an HTTPS proxy, with developers created and ready to connect Claude Code and Cursor. Everything the agent runs is one idempotent script, [`deploy/stack/stack.py`](../../deploy/stack/stack.py), so the work is reviewable and safe to repeat after a failure.

This is a small-team evaluation setup. Read [production readiness](production-readiness.md) before relying on it: the script creates a private CA by default, keeps one Keycloak and one database on one host, and does not configure backups.

## What the administrator provides

| Input | Notes |
|---|---|
| SSH access | A non-root user in the `docker` group on a Linux host with Docker Compose v2, `git`, `python3` (3.9+) and `openssl`. Key-based login; the agent never needs a password. |
| Repository URL | The git URL of this repository (and a ref if not `main`). |
| Domain | Two DNS names pointing at the host: `sso.DOMAIN` and `wk.DOMAIN`. The hostnames become part of every token, so choose them before developers sign in. |
| Developers | A name and services for each, for example `anika:backend,frontend:review` (`review` makes a reviewer). |
| TLS | Either a certificate chain and key valid for both names (`--tls-cert`, `--tls-key`), or accept the private CA the script creates, whose `ca.pem` every developer must trust. |

Ports `8443` (Keycloak) and `8444` (server) are published; restrict them with a firewall to the people who need them. Keycloak's admin console is on `8443`.

## Prompt to give the agent

```text
Set up Workspace Knowledge on the server I give you access to.

SSH: user@host (key already loaded)
Repository: https://github.com/OWNER/workspace-knowledge.git (ref: main)
Domain: example.com   (sso.example.com and wk.example.com already resolve to the host)
Developers: anika:backend,frontend:review  ravi:backend,frontend
TLS: use the private CA the script creates   (or: cert /path/chain.pem, key /path/key.pem)

Follow docs/operations/agent-install.md in the repository exactly. Run only the commands it lists.
Do not print passwords or the contents of credentials.txt in this chat. When done, report the
portal and MCP addresses, the path of ca.pem and credentials.txt on the host, and the result of the
verification step. Stop and ask me before opening firewall ports, changing DNS, or deleting data.
```

## Steps the agent follows

1. **Check the host.** `ssh user@host 'docker compose version && git --version && python3 --version && openssl version'`. Stop and report anything missing; do not install system packages without being asked.
2. **Get the code.** `ssh user@host 'git clone --depth 1 --branch main REPO_URL ~/workspace-knowledge'` (or `git -C ~/workspace-knowledge pull` when it exists).
3. **Bring up the stack.** On the host, in the checkout:

   ```sh
   python3 deploy/stack/stack.py up --domain DOMAIN --bind 0.0.0.0 \
     --app-port 8444 --user anika:backend,frontend:review --user ravi:backend,frontend
   ```

   The first run builds the server image (several minutes), creates certificates and secrets in `~/.local/share/workspace-knowledge/stack` (outside the checkout, directory mode 0700), starts the containers, runs the database migration, creates the restricted application database role, creates the Keycloak users and their memberships, and enables native MCP sign-in for Claude Code, Cursor and VS Code. If a step fails, fix the cause and run the same command again.
4. **Verify.** `python3 deploy/stack/verify.py --domain DOMAIN` signs two of the users in, shares a summary as one and checks the other is told who did it, then withdraws its test card. It expects `PASS:` on the last line. If the host cannot reach its own public names, run it from a machine that can and give it `--state-dir` with a copy of `secrets.json` and `certs/ca.pem`.
5. **Report.** Give the administrator the portal address `https://wk.DOMAIN:8444`, the MCP address `https://wk.DOMAIN:8444/mcp/demo`, and where `credentials.txt` and `certs/ca.pem` are. Passwords reach developers through the administrator's own secure channel, not through the agent's chat.

## Then each developer

Trust `ca.pem` (not needed with a public CA), then follow steps 2 to 5 of the [two-developer test guide](local-docker-test.md) with the real MCP address. Claude Desktop uses the `wk-mcp` bridge ([setup](local-docker-test.md#claude-desktop-the-chat-app-local-or-deployed)), which works against any server, including one behind a private CA; its cloud connector needs a public server and a public certificate. Adding a developer later is the same `stack.py up` command with another `--user`; list every existing user again, because `--user` replaces the default pair. `stack.py down` stops the stack and keeps data; `down --volumes` deletes it.

## Boundaries

- The agent needs shell access to the host and can read the secrets the script writes; give it only to a host and an administrator session you are willing to expose to it, and rotate the Keycloak admin password afterwards.
- It does not set up backups, the retention worker, a collector, semantic search or OpenBao. See the [install guide](install.md), [backup](backup.md) and [hosted portal](hosted-portal.md) guides for those.
- The script never opens firewall ports, edits DNS or installs system packages.
