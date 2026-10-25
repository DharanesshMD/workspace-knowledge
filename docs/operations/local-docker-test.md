# Try it with two developers: local Docker, Claude Code and Cursor

A full local stack (PostgreSQL, Keycloak, the server and an HTTPS proxy, all in Docker) with two named developers. One makes a change in Claude Code and shares it; the other asks Cursor to do something similar and its agent is told, by name, that the first developer already did it. This is a manual evaluation setup on one machine, not a production deployment.

## 1. Start the stack

You need Docker, `python3` and `openssl`. From the repository root:

```sh
docker build -f deploy/Dockerfile --target runtime -t wk-server:local .   # first time only; several minutes
python3 deploy/stack/stack.py up
```

(`up` builds the image itself if it is missing. The explicit build is just easier to watch.) It prints the addresses and is safe to run again: certificates, passwords and users are reused.

| What | Address |
|---|---|
| Portal | `https://wk.localhost:8444` |
| MCP address for product `demo` | `https://wk.localhost:8444/mcp/demo` |
| Keycloak | `https://sso.localhost:8443` |
| Users | `anika` (backend, frontend, reviewer) and `ravi` (backend, frontend) |
| Passwords, Keycloak admin login | `~/.local/share/workspace-knowledge/stack/credentials.txt` (mode 0600) |
| Private CA to trust | `~/.local/share/workspace-knowledge/stack/certs/ca.pem` |

Check the server side without any coding agent (it signs both users in, shares a summary as the first and checks the second is told who did it, then withdraws its test card):

```sh
python3 deploy/stack/verify.py
```

## 2. Trust the local CA

The certificates come from a private CA, so your browser and Cursor must trust it. This adds it to your login keychain and macOS asks you to confirm:

```sh
security add-trusted-cert -r trustRoot -k ~/Library/Keychains/login.keychain-db ~/.local/share/workspace-knowledge/stack/certs/ca.pem
```

Quit and reopen Cursor and the browser afterwards. Firefox uses its own store, so use Safari or Chrome. Claude Code takes the CA from an environment variable instead (step 4).

## 3. Create the two project folders

```sh
deploy/stack/make-demo-folders.sh
```

This creates `~/wk-demo/anika-claude-code` and `~/wk-demo/ravi-cursor`, two clones of a tiny shop repo (`backend/retry.rs` retries a payment three times). Each has the MCP address configured (`.mcp.json` for Claude Code, `.cursor/mcp.json` for Cursor) and a short project rule that tells the agent to check for teammates' work before editing and to share what it did afterwards.

## 4. Developer 1 (Anika) in Claude Code

```sh
cd ~/wk-demo/anika-claude-code
NODE_EXTRA_CA_CERTS=$HOME/.local/share/workspace-knowledge/stack/certs/ca.pem claude
```

1. Approve the project MCP server when Claude Code asks.
2. Run `/mcp`, choose `workspace-knowledge`, then **Authenticate**. Sign in as `anika` (password in `credentials.txt`) and accept the consent screen. The server should show as connected with its tools.
3. Ask for a change, for example:

   > In backend/retry.rs, add an idempotency key per order to the payment retry so retries can't double-charge, and wait 100ms, 200ms, 400ms between attempts.

   The agent should call `knowledge.overlaps` first (nothing yet), edit the file, then call `knowledge.share_session`. If it does not share, say "share this work with the team". The result says `published: true`.

## 5. Developer 2 (Ravi) in Cursor

**Before signing in, make the browser forget Anika.** Both sign-ins go through the same browser and Keycloak keeps a single-sign-on cookie, so Cursor would silently sign in as Anika. Remove the cookies for `sso.localhost` (Safari: Settings → Privacy → Manage Website Data; Chrome: padlock next to the address → Cookies and site data), or use a different default browser for this step. Do not use Keycloak's "sign out": that ends Anika's session and her Claude Code token with it.

```sh
cursor ~/wk-demo/ravi-cursor
```

1. In Cursor, open Settings → MCP (or Tools & MCP), find `workspace-knowledge` and connect. Sign in as `ravi` and accept the consent screen.
2. In agent mode, ask for something similar:

   > Add an idempotency key to the payment retry in backend/retry.rs so retries don't double-charge.

3. Expected: before editing, the agent calls `knowledge.overlaps` and tells you that **anika** already worked on this, what she decided (key per order, not per attempt) and the constraint she recorded. `knowledge.search` and `knowledge.context` return the card but not the author. The name comes from `knowledge.overlaps`, which is why the project rule asks for it.

To confirm who is signed in, open `https://wk.localhost:8444` and look at the account name. The **Connect** page there shows the same MCP address.

## If something is off

| Symptom | Likely cause |
|---|---|
| Browser warns about the certificate | The CA is not trusted yet (step 2), or you are using Firefox. |
| Claude Code: "Failed to connect" or TLS error | Start it with `NODE_EXTRA_CA_CERTS=...` as above. |
| Cursor's agent says the name is `anika` but you are `ravi` (or the other way round) | The browser reused the other developer's Keycloak session. Clear `sso.localhost` cookies and reconnect that client. |
| The agent does not mention a teammate | Agents follow rules unevenly. Ask: "call knowledge.overlaps with a query describing this task". |
| 401 or 403 from the MCP address | The user is not a member of product `demo` (re-run `stack.py up`), or the token has expired (reconnect). |
| 403 "Host header is not allowed" | Server image older than this fix; rebuild `wk-server:local` and `stack.py up`. |

## Look around, reset, stop

```sh
python3 deploy/stack/stack.py status
docker compose -p wk-stack logs -f app        # server log
python3 deploy/stack/stack.py down             # stop, keep data
python3 deploy/stack/stack.py down --volumes   # stop and delete all data
```

Test cards can be withdrawn in the portal (My context, Withdraw knowledge) so the next run starts clean. Add another developer with `python3 deploy/stack/stack.py up --user name:backend,frontend`; users already in `up` must be listed again, as `--user` replaces the default pair.

## What this does not cover

Capture of past conversations (the collector, `wk-agent`) and the Claude Code edit hook are separate and not set up here. The hosted MCP address cannot approve local folders; "connecting a folder" here means opening a project whose MCP config points at the server, and membership in product `demo` decides what each developer can read and share. Whether a model actually calls the tools in Cursor and Claude Code is exactly what this manual test checks: the automated qualification covered sign-in and tool listing, not a model calling a tool.
