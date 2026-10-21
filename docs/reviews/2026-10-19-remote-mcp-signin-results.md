# Remote MCP sign-in: real-client results (2026-10-19)

Scope: can a developer's agent log in once to the hosted MCP at `/mcp/{product}`, using the stdio bridge's new `wk-mcp login` or a client's own remote OAuth? Everything ran against a disposable local stack built by `scripts/keycloak-acceptance.py`: Keycloak 26.4.7 over HTTPS behind a throwaway private CA, `wk-server --production` behind a Caddy HTTPS proxy, `WK_OIDC_AUDIENCE` = `WK_MCP_RESOURCE` = the proxy origin (`https://localhost:PORT`), PostgreSQL 17. One synthetic user, provisioned to one product. Host: macOS on Apple silicon. This is local evidence for one provider; it is not production-provider, real-team or independent-review evidence.

## Result summary

| Client (version) | Result | Notes |
|---|---|---|
| `wk-mcp login` + stdio bridge (this tree) | **Pass** | One login, refresh token in the macOS Keychain, bridge renews silently across restarts, private CA pinned with no fallback, `WK_ACCESS_TOKEN` fallback, offline access. In `keycloak-acceptance.py` (85 checks green). |
| Claude Code 2.1.15, `claude mcp add --transport http` | **Pass, after one fix and one realm step** | Registers itself, signs in (consent screen), lists tools, calls `knowledge.search` (scripted stand-in model), renews an expired token with no prompt. |
| Cursor agent CLI 2026.05.24-dda726e (`agent mcp login`) | **Pass, after a realm step** | Registers itself, signs in, lists all 10 tools, renews silently. Tool *call* not exercised (needs a Cursor account/model). |
| Cursor editor 3.25.9 | **Pass (one user-driven run)** | Tested in the user's own logged-in Cursor, started with `NODE_EXTRA_CA_CERTS`. It registered itself, the sign-in and consent completed, the token exchange succeeded, and the server saw initialize, `tools/list` and tool traffic with 200s. After the 5-minute access token expired, a reconnect renewed it silently: Keycloak logged `REFRESH_TOKEN` with no login and the server then returned 200. The sign-in was driven partly by hand (see notes) and the tool list was seen only in the server's access log, not in Cursor's UI. |
| VS Code (app bundle 1.135.0) | **Partial — sign-in not completed** | Driven in an isolated profile through its debug port (no GUI access needed). Confirmed: it finds the provider, registers itself once Keycloak is set up for it (see below), and reaches its "wants to authenticate" prompt. Not yet completed: the Keycloak login/consent and the token/refresh/tool steps. |

All clients used isolated profiles (a throwaway `CLAUDE_CONFIG_DIR` or `HOME`, a scratch workspace, no user configuration, hooks or conversation stores read or written). Credentials a client left in the macOS Keychain (`Claude Code-credentials-<hash>` for the throwaway config directory) were removed and verified gone.

## What was confirmed rather than assumed

| Expected hurdle | Finding |
|---|---|
| **Dynamic client registration** | Both clients register anonymously, automatically, at the provider's `registration_endpoint` (found through the server's protected-resource metadata, then the issuer's authorization-server metadata). **Keycloak refuses by default** (`Trusted Hosts` policy: "Host not trusted"; with a host list: "URI doesn't match any trusted host"), so a stock realm shows Claude Code "Failed to connect". Claude Code registers one loopback redirect; **Cursor registers three** (`http://localhost:8787/callback`, `https://www.cursor.com/agents/mcp/oauth/callback`, `cursor://anysphere.cursor-mcp/oauth/callback`) and Keycloak requires all of them on the trusted list. Keycloak's *Consent Required* policy also shows the user a consent screen once per registered client. Fix: `deploy/keycloak/enable-remote-mcp-signin.py` with `--trusted-host` for each. |
| **Resource indicators** | Both clients send `resource=https://localhost:PORT/` — the origin **with a trailing slash**, not the MCP URL and not the configured `WK_MCP_RESOURCE` (no slash). Keycloak 26.4.7 ignores `resource` completely (no validation, no effect on `aud`). The audience therefore comes only from a mapper, and a provider that does honour `resource` would have to accept the slash form. Not testable here. |
| **Tokens carry the single required audience** | **No, by default.** A self-registered client got a token with **no `aud` at all** (Claude Code then reported "Server returned 401 after successful authentication"). Keycloak gives registered clients the realm's default scopes only. Fix: a realm default client scope with an audience mapper for `WK_MCP_RESOURCE`, and `roles` (adds `account`) removed from the realm defaults. After it, both clients' tokens have `aud` exactly the resource (a single string) and the server accepts them. The server's single-audience rule was not changed. |
| **Refresh** | Both pass: after the access token expired (realm lifetime shortened to 12 s for the test), a new run obtained a fresh token with no browser and no prompt. Claude Code's log shows "Failed to discover OAuth metadata" in a *proactive* refresh path (it probes the server origin's `/.well-known/oauth-authorization-server`, `openid-configuration` and the MCP path, all 404) but renewal still works through its other path, so it is harmless here. An early Cursor run looked like "no silent refresh"; that was my mistake (running `agent mcp login` twice registered a second client but kept the first client's tokens, so the refresh was refused with "Token client and authorized client don't match"). A clean login renews fine. |
| **Private-CA trust in the client** | Both pass with `NODE_EXTRA_CA_CERTS=<bundle>` (Claude Code's own TLS and its Bun/Node OAuth discovery; the Cursor CLI likewise). Without it a client cannot reach the server or provider. Only the editors' behaviour is unknown. |

## Cursor editor notes

- **Private CA:** `NODE_EXTRA_CA_CERTS` alone was not enough; Cursor's MCP process also needed `http.systemCertificatesNode` and `http.systemCertificates` set to `false` (as VS Code). A real team should trust the CA in the OS store instead.
- **Redirect:** the editor registers `https://www.cursor.com/agents/mcp/oauth/callback` (and the loopback one), so the code returns through cursor.com, which hands it back to the editor. A normal user needs no step here; the test needed a browser that trusts the CA (Chrome did not), so the sign-in was completed with the harness's browser stand-in and the final callback link was opened in the user's Chrome.
- **An unexplained re-login:** a first reconnect after expiry went to "needs authentication" with no refresh attempt. It followed my rewriting `~/.cursor/mcp.json` (same content, different formatting) to try to force a reload; Cursor keys stored tokens by a hash of the server's configuration, so that edit probably made it forget them. Not proven. The clean second run, without touching the file, renewed silently.
- **Not covered:** the Cursor UI's tool list, a real model call, Windows/Linux.

## VS Code findings so far

- **Private CA:** VS Code sets `NODE_USE_SYSTEM_CA=1` for its extension host, so `NODE_EXTRA_CA_CERTS` is ignored ("self signed certificate in certificate chain"). With `http.systemCertificatesNode` and `http.systemCertificates` set to `false` in the profile, the extra CA is honoured. A real team needs the CA in the OS trust store.
- **Registration hosts:** VS Code registers redirects `http://127.0.0.1:33418/`, `https://vscode.dev/redirect` and `https://insiders.vscode.dev/redirect`, and sends a client URI on `code.visualstudio.com`; Keycloak's Trusted Hosts policy refuses until all of those hosts are listed (Keycloak's server log names the offending URL: `KC-SERVICES0102`/`0108`). Until then VS Code shows "Dynamic Client Registration not supported" and asks for a client ID.
- **Scopes:** it registers with `scope=openid` and then asks for `openid offline_access` (or, with no scope list, every scope Keycloak publishes). Keycloak's *Allowed Client Scopes* policy refused `openid`, and a registration that names a scope replaces the realm's default scopes (so the audience scope is not attached). The setup script now allows `openid` and hides the `roles` and `service_account` scopes from the provider metadata (Keycloak answers `invalid_scope` for those two). Whether the audience scope is attached for VS Code's registration, and so whether its token has the right `aud`, is the open question.
- **RFC 9728:** VS Code checks that the metadata's `resource` equals the MCP URL and logs a warning (the server publishes the origin); it carries on.
- **Reaching the prompt:** MCP start, then "The MCP Server Definition wants to authenticate to localhost" appears as a custom dialog only with `window.dialogStyle: custom`.

## Defect found and fixed

**Claude Code 2.1.15 showed no tools.** Its tool-list validation failed at `tools[7].inputSchema.properties.card`. `knowledge.record`'s `card` was an untyped JSON value, which schemars renders as the boolean schema `true` (valid JSON Schema, rejected by that client). It affected the stdio bridge and the hosted endpoint alike. Both now give `card` an object schema; each server has a test that every tool property has an object schema (it failed before the fix). Verified: Claude Code then lists the tools and calls one. The Cursor agent CLI was not affected.

## Other observations (client behaviour, not changed)

- The Cursor agent CLI keeps its tokens in `<HOME>/.cursor/projects/<workspace>/mcp-auth.json` with mode `0644` (readable by other local users, access and refresh token). Claude Code's `.credentials.json` was `0600`.
- Each `claude mcp` health check or login of a new profile registers another client; Keycloak caps anonymous registrations at 200 and does not delete them.
- Claude Code's first-run screens had to be bypassed with a seeded config and a fake API key, and its tool call used a scripted stand-in for the model, because no Anthropic login is used. `scripts/mcp-client-oauth.py` encodes this and may break with other Claude Code versions.

## Not tested

The Cursor and VS Code editors; any provider other than Keycloak 26.4.7; public-CA providers; `wk-mcp login` and its credential store on Linux and Windows; Claude Code or Cursor versions other than those above; a real model calling the tools; what happens after the SSO session ends (the bridge reports "sign-in expired or revoked; run `wk-mcp login` again" in unit tests only); clock skew and provider outages.

## Re-running

```sh
cargo build -p wk-server -p wk-agent -p wk-cli -p wk-mcp
python3 scripts/keycloak-acceptance.py --output /tmp/keycloak-result.json            # ~7 minutes, includes wk-mcp and a native-client simulation
python3 scripts/keycloak-acceptance.py --stack-only /tmp/wk-stack.json &              # leaves the stack up for clients
python3 scripts/mcp-client-oauth.py --stack /tmp/wk-stack.json --output /tmp/clients.json   # 11 checks, then stop the stack with SIGTERM
```

Last run of `scripts/mcp-client-oauth.py`: Claude Code 2.1.15 6/6, Cursor agent CLI 2026.05.24-dda726e 5/5, plus the credential-file note above.
