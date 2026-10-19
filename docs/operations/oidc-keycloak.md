# Keycloak identity setup and qualification

Workspace Knowledge verifies standard OIDC. This page records how to configure Keycloak and what was qualified. It is the only provider qualified so far; other standards-compliant providers should work but are untested by this project.

**Qualified:** Keycloak 26.4.7 (`quay.io/keycloak/keycloak:26.4`), running over HTTPS behind a private CA, with the server in `--production` mode behind an HTTPS reverse proxy. Run date and results are in the [production readiness ledger](production-readiness.md). This is a disposable local qualification, not evidence for any other provider, a public-CA hosted provider, or a real team.

## Realm and client configuration

| Item | Setting |
|---|---|
| Portal client | Confidential, standard flow only, PKCE method `S256`, exact redirect URI `WK_CONSOLE_ORIGIN/auth/callback`. Give the client secret to `WK_BROWSER_CLIENT_SECRET`. |
| Collector client | Public, standard flow only, PKCE `S256`, redirect URI `http://127.0.0.1/callback`. Keycloak accepts the collector's ephemeral loopback port against this entry. Set its ID as `WK_COLLECTOR_CLIENT_ID`. |
| API audience | Add a **Hardcoded audience** mapper (include in access token only) with the value of `WK_OIDC_AUDIENCE`. |
| Collector audience | A separate mapper with the value of `WK_COLLECTOR_AUDIENCE`; it must differ from the API audience. |
| Client scopes | Keep only `basic` (it supplies `sub`). Remove Keycloak's default scopes from clients that obtain bearer tokens. |

**Single-audience rule.** The server accepts a bearer token only when `aud` is exactly the configured audience. A stock Keycloak client also receives the `roles` scope, which adds `account` to `aud`; every such token is rejected (verified: `aud=["wk-api","account"]` returns 401). Remove the `roles` client scope or its audience-resolve mapper, or issue tokens from a client without default scopes.

The server never derives membership from provider claims. Map each Keycloak user explicitly with `wk-server provision`, using the realm issuer URL and the user's stable Keycloak `sub`.

## Private certificate authority

Out of the box the server trusts only the public CA roots built into its binary. If your provider uses a private or internal CA, set `WK_OIDC_CA_FILE` to a PEM bundle:

- It applies only to requests to the identity provider (discovery, signing keys and token exchange) and **replaces** the public roots for those requests, so it also pins the provider to your CA.
- The file must contain only valid certificates (maximum 64 KiB). A malformed, empty or wrong-CA bundle stops startup with a clear error.
- It does not affect the TLS certificate your reverse proxy presents to browsers.

### Native collector

The collector (`wk-agent`) also trusts only public roots by default. For a server and identity provider behind a private CA, pass the same bundle once when connecting:

```sh
wk-agent --state-dir "$HOME/.local/share/workspace-knowledge/collector" device connect \
  --binding /absolute/path/from/binding_file.json --service backend \
  --server https://knowledge.internal --ca-file /path/to/private-ca.pem --yes
```

- `--ca-file` is validated like `WK_OIDC_CA_FILE` (certificates only, maximum 64 KiB, malformed or empty rejected) and requires an HTTPS server. Nothing is read from an environment variable.
- The bundle is copied to `server-ca.pem` in the collector's private state directory. Every later request to the hosted server and its identity provider (discovery, keys, token exchange, signed device requests, `device login`) pins that copy and **replaces** the public roots for them. Local model and loopback traffic are unaffected.
- Without `--ca-file` the public roots apply and no bundle is stored. A stored bundle that is later unreadable, replaced by a symlink or invalid makes requests fail with `stored CA bundle invalid`; the collector never falls back to public roots. `wk-agent device status` reports `server_trust` as `private_ca` or `public_roots`.
- The state directory is private (0700), so the bundle is protected like the rest of the collector's configuration. To change the bundle (for example after a CA rotation), connect again with a new private state directory.
- The optional `wk search` command and the `wk-mcp` bridge, which use environment tokens, read an explicitly set `WK_SERVER_CA_FILE` with the same rules; unset or empty means public roots.

With Docker Compose, mount the bundle read-only and set the variable to its in-container path, for example with a compose override file.

## Signing-key rotation

The server refreshes the provider's signing keys every 60 seconds and fails closed if refreshes keep failing (cached keys expire after five minutes). Measured on Keycloak:

- A token signed by a newly promoted key was rejected until the next refresh and accepted about 49 s after promotion.
- A token signed by a deleted key was rejected about 61 s after deletion.
- Tokens signed by the previous key stay valid for as long as the provider still publishes it.

Rotate safely: publish the new key at a lower priority (passive) first, wait at least two minutes, then raise its priority, and remove the old key only after the longest-lived token it signed has expired.

## Re-running the qualification

The script creates and removes its own throwaway CA, Keycloak, PostgreSQL and HTTPS proxy containers. It needs Docker, OpenSSL and a built server:

```sh
cargo build -p wk-server -p wk-agent -p wk-cli
python3 scripts/keycloak-acceptance.py --output /tmp/keycloak-result.json
```

It checks: browser login with PKCE, state and nonce and the registered HTTPS callback; secure session cookies; replay and forged-state rejection; logout and CSRF; wrong, extra, missing and default-scope audiences; corrupted, unsigned and expired tokens; the native collector's authorization-code flow with an ephemeral loopback port and signed device requests; and, on macOS, the real `wk-agent device connect --ca-file`, `device poll` and `device login` against the private-CA Keycloak and Caddy stack, including refusal without the bundle, with an unrelated or malformed bundle, and after the stored bundle is replaced or corrupted (this step uses synthetic macOS keychain entries and is reported as skipped elsewhere); startup failures for an untrusted or malformed CA, plain-HTTP key URL, issuer mismatch and an unreachable provider; and key rotation. The output holds only pass/fail results, never tokens or credentials.

## Not covered

Other providers; a provider with a public-CA certificate; provider-side logout or session revocation (the server's session ends on its own logout or expiry, and bearer tokens stay valid until they expire); real browser rendering of the Keycloak login page (the script submits the form directly); multiple users, groups or organizations; provider outage and recovery drills; and the native collector on Linux or Windows against a private CA (the end-to-end check runs on macOS only; the TLS behavior is also covered by cross-platform unit tests).
