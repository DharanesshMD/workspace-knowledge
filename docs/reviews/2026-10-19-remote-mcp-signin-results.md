# Remote MCP sign-in: real-client results (2026-10-19)

Scope: can a developer's agent log in once to the hosted MCP at `/mcp/{product}`, using the stdio bridge's new `wk-mcp login` or a client's own remote OAuth? Everything ran against a disposable local stack built by `scripts/keycloak-acceptance.py`: Keycloak 26.4.7 over HTTPS behind a throwaway private CA, `wk-server --production` behind a Caddy HTTPS proxy, `WK_OIDC_AUDIENCE` = `WK_MCP_RESOURCE` = the proxy origin (`https://localhost:PORT`), PostgreSQL 17. One synthetic user, provisioned to one product. Host: macOS on Apple silicon. This is local evidence for one provider; it is not production-provider, real-team or independent-review evidence.

## Result summary

| Client (version) | Result | Notes |
|---|---|---|
| `wk-mcp login` + stdio bridge (this tree) | **Pass** | One login, refresh token in the macOS Keychain, bridge renews silently across restarts, private CA pinned with no fallback, `WK_ACCESS_TOKEN` fallback, offline access. In `keycloak-acceptance.py` (85 checks green). |
| Claude Code 2.1.15, `claude mcp add --transport http` | **Pass, after one fix and one realm step** | Registers itself, signs in (consent screen), lists tools, calls `knowledge.search` (scripted stand-in model), renews an expired token with no prompt. |
| Cursor agent CLI 2026.05.24-dda726e (`agent mcp login`) | **Pass, after a realm step** | Registers itself, signs in, lists all 10 tools, renews silently. Tool *call* not exercised (needs a Cursor account/model). |
| Cursor editor 3.25.9 | **Pass (one user-driven run)** | Tested in the user's own logged-in Cursor, started with `NODE_EXTRA_CA_CERTS`. It registered itself, the sign-in and consent completed, the token exchange succeeded, and the server saw initialize, `tools/list` and tool traffic with 200s. After the 5-minute access token expired, a reconnect renewed it silently: Keycloak logged `REFRESH_TOKEN` with no login and the server then returned 200. The sign-in was driven partly by hand (see notes) and the tool list was seen only in the server's access log, not in Cursor's UI. |
| VS Code 1.142.0 (app bundle; the earlier partial run was 1.135.0) | **Pass for sign-in, tool listing and silent renewal, with one realm requirement; a tool call was not exercised** | Run 2026-10-22 by `scripts/mcp-client-oauth.py --only vscode` in an isolated profile (no GUI interaction needed). It registered itself, showed its "wants to authenticate" dialog, signed in through Keycloak with consent, exchanged the code, and the server answered initialize, the initialized notification and `tools/list` (200, 202, 200); VS Code listed all 10 tools. After the access token expired, restarting the MCP server renewed it with one Keycloak `REFRESH_TOKEN` event, no second login and no browser. **Requires an access-token lifespan above 5 minutes** (see "VS Code results"); at Keycloak's default 300 s it fires dozens of refreshes after each sign-in. Not exercised: a model calling a tool (needs a GitHub Copilot sign-in). |

All clients used isolated profiles (a throwaway `CLAUDE_CONFIG_DIR` or `HOME`, a scratch workspace, no user configuration, hooks or conversation stores read or written). Credentials a client left in the macOS Keychain (`Claude Code-credentials-<hash>` for the throwaway config directory) were removed and verified gone.

## What was confirmed rather than assumed

| Expected hurdle | Finding |
|---|---|
| **Dynamic client registration** | Both clients register anonymously, automatically, at the provider's `registration_endpoint` (found through the server's protected-resource metadata, then the issuer's authorization-server metadata). **Keycloak refuses by default** (`Trusted Hosts` policy: "Host not trusted"; with a host list: "URI doesn't match any trusted host"), so a stock realm shows Claude Code "Failed to connect". Claude Code registers one loopback redirect; **Cursor registers three** (`http://localhost:8787/callback`, `https://www.cursor.com/agents/mcp/oauth/callback`, `cursor://anysphere.cursor-mcp/oauth/callback`) and Keycloak requires all of them on the trusted list. Keycloak's *Consent Required* policy also shows the user a consent screen once per registered client. Fix: `deploy/keycloak/enable-remote-mcp-signin.py` with `--trusted-host` for each. |
| **Resource indicators** | Both clients send `resource=https://localhost:PORT/` — the origin **with a trailing slash**, not the MCP URL and not the configured `WK_MCP_RESOURCE` (no slash). Keycloak 26.4.7 ignores `resource` completely (no validation, no effect on `aud`). The audience therefore comes only from a mapper, and a provider that does honour `resource` would have to accept the slash form. Not testable here. |
| **Tokens carry the single required audience** | **No, by default.** A self-registered client got a token with **no `aud` at all** (Claude Code then reported "Server returned 401 after successful authentication"). Keycloak gives registered clients the realm's default scopes only. Fix: a realm default client scope with an audience mapper for `WK_MCP_RESOURCE`, and `roles` (adds `account`) removed from the realm defaults. After it, both clients' tokens have `aud` exactly the resource (a single string) and the server accepts them. The server's single-audience rule was not changed. |
| **Refresh** | Both pass: after the access token expired (realm lifetime shortened to 12 s for the test), a new run obtained a fresh token with no browser and no prompt. Claude Code's log shows "Failed to discover OAuth metadata" in a *proactive* refresh path (it probes the server origin's `/.well-known/oauth-authorization-server`, `openid-configuration` and the MCP path, all 404) but renewal still works through its other path, so it is harmless here. An early Cursor run looked like "no silent refresh"; that was my mistake (running `agent mcp login` twice registered a second client but kept the first client's tokens, so the refresh was refused with "Token client and authorized client don't match"). A clean login renews fine. |
| **Private-CA trust in the client** | Both pass with `NODE_EXTRA_CA_CERTS=<bundle>` (Claude Code's own TLS and its Bun/Node OAuth discovery; the Cursor CLI likewise). Without it a client cannot reach the server or provider. The Cursor and VS Code editors both needed more (see their sections). |

## Cursor editor notes

- **Private CA:** `NODE_EXTRA_CA_CERTS` alone was not enough; Cursor's MCP process also needed `http.systemCertificatesNode` and `http.systemCertificates` set to `false` (as VS Code). A real team should trust the CA in the OS store instead.
- **Redirect:** the editor registers `https://www.cursor.com/agents/mcp/oauth/callback` (and the loopback one), so the code returns through cursor.com, which hands it back to the editor. A normal user needs no step here; the test needed a browser that trusts the CA (Chrome did not), so the sign-in was completed with the harness's browser stand-in and the final callback link was opened in the user's Chrome.
- **An unexplained re-login:** a first reconnect after expiry went to "needs authentication" with no refresh attempt. It followed my rewriting `~/.cursor/mcp.json` (same content, different formatting) to try to force a reload; Cursor keys stored tokens by a hash of the server's configuration, so that edit probably made it forget them. Not proven. The clean second run, without touching the file, renewed silently.
- **Not covered:** the Cursor UI's tool list, a real model call, Windows/Linux.

## VS Code results (1.142.0, run 2026-10-22)

Automated by `scripts/mcp-client-oauth.py --only vscode` against the `--stack-only` stack: VS Code runs from the real app bundle with a throwaway `--user-data-dir` and `--extensions-dir` under `/tmp`, its debug ports open, `NODE_EXTRA_CA_CERTS` set, and a workspace `.vscode/mcp.json` with one `http` server. The harness hooks `shell.openExternal` in VS Code's main process (so the sign-in URL is captured, not opened in the user's browser), presses command-palette keys through the debug port to start and restart the server, plays the browser with the same stand-in as the other clients, and reads Keycloak's recorded events and the proxy's access log. Hashes of the user's own VS Code `settings.json`, `mcp.json` and `argv.json` were identical before and after, and the profile was deleted. (`~/.vscode/argv.json` is shared by every VS Code instance whatever its `--user-data-dir`: an early manual run whose profile set `telemetry.telemetryLevel: off` flipped its `enable-crash-reporter` flag. It was restored byte for byte, and the harness profile now sets no telemetry option.) Final run at a 330 s access-token lifespan: 7/7 checks (several earlier 330 s runs only exposed harness fixes); a run at 300 s: 6/7 (the refresh-burst check, below).

| Step | Result |
|---|---|
| Finds the provider, registers itself | Pass once the realm is set up (below). It registers as a public client with redirects `http://127.0.0.1:33418/`, `https://vscode.dev/redirect`, `https://insiders.vscode.dev/redirect`. |
| "The MCP Server Definition 'wk' wants to authenticate to localhost:PORT" dialog | Appears and continues on Allow. The harness uses `window.dialogStyle: custom` only so it can be clicked through the debug port; a user sees the normal dialog. In one of five harness and debug runs the sign-in began with no such dialog; why is not known. |
| Authorization request | PKCE S256, loopback redirect `http://127.0.0.1:PORT/`, `resource=https://localhost:PORT` (the origin, **no** trailing slash, unlike Claude Code and Cursor), and every scope the provider advertises: `openid microprofile-jwt email basic address phone profile organization wk-mcp-resource acr web-origins offline_access`. Because it asks for `wk-mcp-resource`, the audience scope is attached; the earlier open question is answered. |
| Keycloak login and consent | Pass (one `LOGIN`, one `CODE_TO_TOKEN`, one `CLIENT_REGISTER` event). |
| Token accepted by the server | Pass. The server requires exactly one audience equal to the resource and answered 200, so the token's `aud` is right; the token itself is held in VS Code's encrypted storage and was not read. |
| Server traffic | `POST /mcp/demo` 200 (initialize), `POST` 202 (initialized notification), `GET /mcp/demo` 405 (the optional server-push stream, which this stateless server does not offer; VS Code carries on), `POST` 200 (`tools/list`). VS Code logged "Discovered 10 tools". |
| Tool names | Each name contains a dot, which VS Code's rule `[a-z0-9_-]` rejects. It logs a warning per tool and rewrites the name sent to the model (`knowledge.search` becomes `knowledge_search`), keeping the original name for the call to the server. Source-read, not exercised. |
| Tool call | **Not exercised.** It needs a model, which needs a GitHub Copilot sign-in; none was used. |
| Silent renewal | Pass. The access token expired (330 s lifespan, harness waited 340 s), the server was restarted from the command palette, Keycloak recorded a `REFRESH_TOKEN` and no second `LOGIN`, no second Keycloak URL was opened, and the server answered 200. |

### Access-token lifespan must be above 5 minutes

VS Code's authentication provider (the extension-host code in 1.142.0) refreshes any session whose remaining lifetime is under 300 s whenever it is asked for a session, and a refresh changes the session list, which asks again. Measured:

- **40 s lifespan (manual run):** about 14,500 "about to expire, refreshing" log lines in two minutes, the number of stored sessions doubling each round from 1 to 256; the MCP `initialize` waited about 40 s before it went through.
- **300 s lifespan, Keycloak's default and the starter realm's value (harness):** 41 `REFRESH_TOKEN` events immediately after sign-in and 52 by the end of the run. Everything still worked.
- **330 s lifespan:** the check for more than 10 refresh events passed; an earlier 330 s run recorded exactly one refresh event, the one after expiry.

Claude Code and Cursor did not show this (Cursor's editor run used the 5-minute default). So VS Code users need a realm Access Token Lifespan above 5 minutes (10 is reasonable). That lengthens how long a stolen access token works, so it is a deliberate choice for the operator and `deploy/keycloak/wk-realm.json` is **not** changed. `enable-remote-mcp-signin.py` now prints a warning when the realm's lifespan is 300 s or less.

### Earlier findings (1.135.0, still true)

- **Private CA:** VS Code sets `NODE_USE_SYSTEM_CA=1` for its extension host, so `NODE_EXTRA_CA_CERTS` is ignored ("self signed certificate in certificate chain"). With `http.systemCertificatesNode` and `http.systemCertificates` set to `false` in the profile, the extra CA is honoured. A real team needs the CA in the OS trust store, which the harness does not do.
- **Registration hosts:** Keycloak's Trusted Hosts policy must list `vscode.dev`, `insiders.vscode.dev`, `code.visualstudio.com` (its client URI) and `127.0.0.1` (Keycloak's server log names the offending URL: `KC-SERVICES0102`/`0108`). Until then VS Code shows "Dynamic Client Registration not supported" and asks for a client ID.
- **Scopes:** it registers with `scope=openid`, so the *Allowed Client Scopes* policy must allow `openid`, and it asks for every advertised scope, so the `roles` and `service_account` scopes must not be advertised (Keycloak answers `invalid_scope` for them). `enable-remote-mcp-signin.py` does all of this.
- **RFC 9728:** VS Code checks that the metadata's `resource` equals the MCP URL and logs a warning (the server publishes the origin, which does not match); it carries on, and also probes `/.well-known/oauth-protected-resource/mcp/demo` (404) first.
- **Other local URL:** VS Code also opens `http://127.0.0.1:PORT/signin?nonce=...` (a local sign-in page of VS Code's own, most likely the GitHub sign-in that Chat uses; not confirmed) in the same session; it is not part of the Keycloak sign-in.
- **A server added after VS Code has started** is not started by VS Code on its own; the harness starts it from the command palette ("MCP: List Servers", the server, "Start Server"). A server in a workspace's `.vscode/mcp.json` at launch started by itself in the manual runs.

## Defect found and fixed

**Claude Code 2.1.15 showed no tools.** Its tool-list validation failed at `tools[7].inputSchema.properties.card`. `knowledge.record`'s `card` was an untyped JSON value, which schemars renders as the boolean schema `true` (valid JSON Schema, rejected by that client). It affected the stdio bridge and the hosted endpoint alike. Both now give `card` an object schema; each server has a test that every tool property has an object schema (it failed before the fix). Verified: Claude Code then lists the tools and calls one. The Cursor agent CLI was not affected.

## Other observations (client behaviour, not changed)

- The Cursor agent CLI keeps its tokens in `<HOME>/.cursor/projects/<workspace>/mcp-auth.json` with mode `0644` (readable by other local users, access and refresh token). Claude Code's `.credentials.json` was `0600`.
- Each `claude mcp` health check or login of a new profile registers another client; Keycloak caps anonymous registrations at 200 and does not delete them.
- Claude Code's first-run screens had to be bypassed with a seeded config and a fake API key, and its tool call used a scripted stand-in for the model, because no Anthropic login is used. `scripts/mcp-client-oauth.py` encodes this and may break with other Claude Code versions.

## Not tested

A model calling a tool through VS Code or Cursor; VS Code on Windows or Linux or with the CA in the OS trust store; any provider other than Keycloak 26.4.7; public-CA providers; `wk-mcp login` and its credential store on Linux and Windows; Claude Code or Cursor versions other than those above; a real model calling the tools; what happens after the SSO session ends (the bridge reports "sign-in expired or revoked; run `wk-mcp login` again" in unit tests only); clock skew and provider outages.

## Re-running

```sh
cargo build -p wk-server -p wk-agent -p wk-cli -p wk-mcp
python3 scripts/keycloak-acceptance.py --output /tmp/keycloak-result.json            # ~7 minutes, includes wk-mcp and a native-client simulation
python3 scripts/keycloak-acceptance.py --stack-only /tmp/wk-stack.json &              # leaves the stack up for clients
python3 scripts/mcp-client-oauth.py --stack /tmp/wk-stack.json --output /tmp/clients.json   # Claude Code, Cursor agent CLI and VS Code, then stop the stack with SIGTERM
```

Last run of `scripts/mcp-client-oauth.py`: Claude Code 2.1.15 6/6, Cursor agent CLI 2026.05.24-dda726e 5/5 (2026-10-19, plus the credential-file note above); VS Code 1.142.0 `--only vscode` 7/7 at the default `--vscode-lifespan 330` (2026-10-22; `--vscode-lifespan 300` fails only the refresh-burst check). The VS Code run takes about 7 minutes, opens a VS Code window, and expects the app at `/Applications/Visual Studio Code.app` (`--vscode PATH` to change it).
