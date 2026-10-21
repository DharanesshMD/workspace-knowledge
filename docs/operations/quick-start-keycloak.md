# Quick start: Keycloak, Docker Compose and the MCP bridge

One page from nothing to "a developer's AI agent reads the team's knowledge after one login". It uses the provider that has been qualified (Keycloak, see [setup and qualification](oidc-keycloak.md)) and the starter realm in [`deploy/keycloak/wk-realm.json`](../../deploy/keycloak/wk-realm.json). This is a small-team starting point, not a certified deployment: read the [production readiness ledger](production-readiness.md) first.

You need a Linux Docker host, two DNS names with TLS certificates (a private CA is fine), and macOS developer machines with the tools installed ([install guide](install.md#install-the-tools)). Examples use `https://knowledge.example` for the server and `https://sso.example:8443` for Keycloak.

## 1. Start Keycloak with the starter realm

The starter realm has the three clients Workspace Knowledge needs and nothing else (no users; you add them). Its settings are the ones the [Keycloak guide](oidc-keycloak.md#realm-and-client-configuration) requires, notably **no stock `account` audience**: the server accepts a token only when `aud` is exactly one configured audience.

| Client | Kind | Used by | Token audience |
|---|---|---|---|
| `wk-portal` | confidential, PKCE `S256` | the web console | none (browser session only) |
| `wk-collector` | public, PKCE `S256`, loopback redirect | `wk-agent device connect` | `WK_COLLECTOR_AUDIENCE` |
| `wk-mcp` | public, PKCE `S256`, loopback redirect, optional `offline_access` | `wk-mcp login` | `WK_OIDC_AUDIENCE` |

Set the audience to the server's public origin: remote MCP requires `WK_OIDC_AUDIENCE` and `WK_MCP_RESOURCE` to be the same HTTPS origin with no trailing slash. Put these in an environment file next to the Compose files (the realm file reads the same names through `${...}` placeholders, so nothing is copied by hand):

```sh
# .env — keep it out of version control and readable only by the operator
WK_CONSOLE_ORIGIN=https://knowledge.example
WK_OIDC_AUDIENCE=https://knowledge.example
WK_MCP_RESOURCE=https://knowledge.example
WK_COLLECTOR_AUDIENCE=wk-collector
WK_BROWSER_CLIENT_ID=wk-portal
WK_BROWSER_CLIENT_SECRET=<random, from your secret manager>
WK_COLLECTOR_CLIENT_ID=wk-collector
WK_MCP_CLIENT_ID=wk-mcp
WK_KEYCLOAK_ORIGIN=https://sso.example:8443
WK_KEYCLOAK_CERT=/etc/wk/sso.crt        # certificate chain, PEM
WK_KEYCLOAK_KEY=/etc/wk/sso.key         # private key, PEM
WK_KEYCLOAK_ADMIN_PASSWORD=<random>
WK_KEYCLOAK_DB_PASSWORD=<random>
```

```sh
docker compose --env-file .env -f deploy/compose/compose.yaml -f deploy/compose/keycloak.yaml up -d keycloak
```

The realm is imported on the first start only; editing the file later does not change an existing realm. Open `https://sso.example:8443`, sign in as `admin`, and in realm **wk** create one user per developer. Copy each user's **ID** (this is the token `sub`). Then change the temporary bootstrap admin to a permanent one, as Keycloak instructs.

## 2. Start the server

Follow [Host a team server](install.md#host-a-team-server) for the database, the restricted `wk_app` role and the HTTPS reverse proxy. In addition to the variables listed there, set:

```sh
WK_OIDC_ISSUER=https://sso.example:8443/realms/wk
WK_OIDC_JWKS_URL=https://sso.example:8443/realms/wk/protocol/openid-connect/certs
WK_OIDC_CA_FILE=/path/in/container/ca.pem   # only if Keycloak uses a private CA; mount it read-only
```

Give each developer access explicitly, using the realm issuer and the user ID from step 1 (the server never infers membership from claims). Run this with the operator database connection, as described in the install guide, not the application's:

```sh
echo '{"issuer":"https://sso.example:8443/realms/wk","subject":"<keycloak user id>","org":"acme","user":"alice","product":"PRODUCT","services":["backend"],"can_review":false}' | wk-server provision
```

Check the server is ready for MCP: `curl https://knowledge.example/.well-known/oauth-protected-resource` should return your origin as `resource`, the Keycloak realm as the one authorization server, and `wk_mcp_client_id`.

## 3. Sign in once and connect an agent

On each developer's Mac, with the tools installed:

```sh
wk-mcp login --server https://knowledge.example --ca-file /path/to/private-ca.pem   # omit --ca-file for a public CA
```

The system browser opens Keycloak; sign in. Only a refresh token is stored, in the macOS Keychain, never in a file, argument or log. `wk-mcp status --server …` shows whether a sign-in is stored and `wk-mcp logout --server …` removes it. The default realm lets the sign-in last up to 24 hours (8 hours idle); then run `wk-mcp login` again. `wk-mcp login --offline-access` asks Keycloak for a long-lived offline token (Keycloak's default is 30 days idle) if you prefer fewer logins.

Add the bridge to the agent. It reads no token: it renews silently from the stored sign-in.

```sh
claude mcp add workspace-knowledge --scope user \
  -e WK_SERVER_URL=https://knowledge.example -e WK_PRODUCT=PRODUCT \
  -e WK_SERVER_CA_FILE=/path/to/private-ca.pem -- wk-mcp
```

Drop the `WK_SERVER_CA_FILE` line for a public CA. For VS Code and Cursor put the same command and environment in `.vscode/mcp.json` or `.cursor/mcp.json` (see [VS Code and GitHub Copilot MCP](../adapters/setup.md#vs-code-and-github-copilot-mcp)). `WK_ACCESS_TOKEN` still works as a fallback when no sign-in is stored, which is how CI and loopback development use it.

Then connect capture: [Join a team server](install.md#join-a-team-server).

## Optional: native remote MCP, no bridge

Claude Code and the Cursor agent CLI can sign in to `https://knowledge.example/mcp/PRODUCT` themselves. They register themselves with Keycloak (dynamic client registration), which Keycloak refuses until you run the one-time setup below. It lets only clients with loopback (and the listed) redirect URIs register, makes every such client's token carry exactly the MCP audience, and keeps Keycloak's consent screen:

```sh
KC_ADMIN_PASSWORD=<keycloak admin password> python3 deploy/keycloak/enable-remote-mcp-signin.py \
  --keycloak https://sso.example:8443 --audience https://knowledge.example --ca-file /path/to/private-ca.pem \
  --trusted-host localhost --trusted-host 127.0.0.1 --trusted-host www.cursor.com --trusted-host anysphere.cursor-mcp
claude mcp add --transport http workspace-knowledge https://knowledge.example/mcp/PRODUCT   # then /mcp, Authenticate
```

Claude Code needs `NODE_EXTRA_CA_CERTS=/path/to/private-ca.pem` in its environment for a private CA. Anyone who can reach Keycloak can then register a loopback client, but the user still has to sign in and consent, and the server still requires an explicit membership. Registrations accumulate in the realm (Keycloak caps anonymous registrations at 200). Per-client results, including what was not tested (VS Code, the Cursor editor), are in the [real-client sign-in results](../reviews/2026-10-19-remote-mcp-signin-results.md). Use the bridge when a client cannot complete this flow.

## If it does not work

| Symptom | Cause |
|---|---|
| `/mcp` returns 401 after sign-in | The token's `aud` is not exactly `WK_OIDC_AUDIENCE`. `wk-mcp login` refuses such a token and prints the audiences; fix the client's audience mapper (the stock `roles` scope adds `account`). |
| `wk-mcp login`: "server unavailable or not trusted" | The server or Keycloak certificate does not chain to `--ca-file`; a configured bundle replaces the public roots, so it must cover both. |
| `wk-mcp login`: "no sign-in client" | The server has no `WK_MCP_CLIENT_ID`; set it or pass `--client-id`. |
| `wk-mcp login`: "does not offer remote MCP sign-in" | `WK_MCP_RESOURCE` is unset or not equal to `WK_OIDC_AUDIENCE`. |
| Bridge says "sign-in expired or revoked" | Run `wk-mcp login` again; the SSO session ended or was revoked. |
| Native client: "Trusted Hosts … not trusted" | Run the optional setup above; add the client's redirect host with `--trusted-host`. |
