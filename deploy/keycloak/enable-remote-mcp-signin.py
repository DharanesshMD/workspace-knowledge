#!/usr/bin/env python3
"""Let native MCP clients (Claude Code, VS Code, Cursor) sign in to a Keycloak realm that fronts
Workspace Knowledge, by letting them register themselves (RFC 7591 dynamic client registration)
and by giving every such client a token the server accepts.

What it changes in the realm (all idempotent; see docs/operations/quick-start-keycloak.md):
  1. creates the client scope `wk-mcp-resource` whose only effect is an audience mapper for the
     server's MCP resource, and makes it a realm default scope, so a client that registers itself
     gets exactly that audience;
  2. removes the stock `roles` scope from the realm defaults, which would add the `account`
     audience and make the server reject every token;
  3. sets the anonymous "Trusted Hosts" registration policy so only clients whose redirect URIs
     (and client/logo/policy URIs) are on the given hosts (default: localhost and 127.0.0.1) can
     register;
  4. lets registrations name the plain `openid` scope, and stops the provider advertising the
     `roles` and `service_account` scopes (VS Code asks for all advertised scopes; Keycloak refuses
     those two for an ordinary client).
Keycloak's own "Consent Required", "Full Scope Disabled" and "Max Clients" anonymous policies stay.
It does not touch clients you already created, and it never prints credentials.

The admin password is read from the KC_ADMIN_PASSWORD environment variable, never from argv.
"""
import argparse, json, os, ssl, sys, urllib.error, urllib.parse, urllib.request

SCOPE = "wk-mcp-resource"
TRUSTED_HOSTS = "org.keycloak.services.clientregistration.policy.ClientRegistrationPolicy"


class Admin:
    def __init__(self, base, realm, ca_file):
        parsed = urllib.parse.urlsplit(base)
        loopback = parsed.hostname in ("localhost", "127.0.0.1", "::1")
        if parsed.scheme != "https" and not (parsed.scheme == "http" and loopback):
            raise SystemExit("the Keycloak URL must be https:// (plain http only to localhost)")
        if parsed.path not in ("", "/") or parsed.query or parsed.fragment or parsed.username:
            raise SystemExit("give the Keycloak origin only, for example https://sso.example")
        self.base, self.realm, self.token = base.rstrip("/"), realm, None
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        if ca_file:  # a private CA replaces the system roots; it is never added to them
            context.load_verify_locations(cafile=ca_file)
        else:
            context.load_default_certs()
        self.opener = urllib.request.build_opener(urllib.request.HTTPSHandler(context=context), urllib.request.ProxyHandler({}))

    def call(self, method, path, body=None, form=None):
        data, headers = None, {}
        if form is not None:
            data, headers["Content-Type"] = urllib.parse.urlencode(form).encode(), "application/x-www-form-urlencoded"
        elif body is not None:
            data, headers["Content-Type"] = json.dumps(body).encode(), "application/json"
        if self.token:
            headers["Authorization"] = "Bearer " + self.token
        try:
            with self.opener.open(urllib.request.Request(self.base + path, data=data, headers=headers, method=method), timeout=20) as r:
                raw = r.read()
        except urllib.error.HTTPError as e:
            raise SystemExit(f"Keycloak refused {method} {path.split('?')[0]}: HTTP {e.code}")
        except OSError as e:
            raise SystemExit(f"cannot reach Keycloak: {e}")
        return json.loads(raw) if raw else None

    def login(self, user, password):
        reply = self.call("POST", "/realms/master/protocol/openid-connect/token",
                          form={"grant_type": "password", "client_id": "admin-cli", "username": user, "password": password})
        self.token = reply["access_token"]

    def realm_path(self, suffix):
        return f"/admin/realms/{urllib.parse.quote(self.realm)}{suffix}"


def enable(admin, audience, hosts, require_trusted_source):
    """Apply the three changes; returns what it did, for the operator's log."""
    done = []
    scopes = admin.call("GET", admin.realm_path("/client-scopes"))
    scope = next((s for s in scopes if s["name"] == SCOPE), None)
    if scope is None:
        admin.call("POST", admin.realm_path("/client-scopes"), {
            "name": SCOPE, "protocol": "openid-connect",
            "description": "Audience for the Workspace Knowledge API and MCP resource.",
            "attributes": {"include.in.token.scope": "false", "display.on.consent.screen": "false"},
            "protocolMappers": [{"name": "aud-mcp-resource", "protocol": "openid-connect", "protocolMapper": "oidc-audience-mapper",
                                 "config": {"included.custom.audience": audience, "access.token.claim": "true", "id.token.claim": "false"}}]})
        scopes = admin.call("GET", admin.realm_path("/client-scopes"))
        scope = next(s for s in scopes if s["name"] == SCOPE)
        done.append(f"created client scope {SCOPE}")
    else:
        mapper = next((m for m in scope.get("protocolMappers", []) if m["protocolMapper"] == "oidc-audience-mapper"), None)
        if not mapper or mapper["config"].get("included.custom.audience") != audience:
            raise SystemExit(f"client scope {SCOPE} exists with a different audience; fix or delete it first")
    defaults = admin.call("GET", admin.realm_path("/default-default-client-scopes"))
    if SCOPE not in [d["name"] for d in defaults]:
        admin.call("PUT", admin.realm_path(f"/default-default-client-scopes/{scope['id']}"))
        done.append(f"made {SCOPE} a realm default scope")
    for stale in ("roles",):
        for d in defaults:
            if d["name"] == stale:
                admin.call("DELETE", admin.realm_path(f"/default-default-client-scopes/{d['id']}"))
                done.append(f"removed {stale} from the realm default scopes (it adds the account audience)")
    # VS Code asks for every scope the provider lists, and Keycloak refuses a client these two (it
    # answers invalid_scope), so keep them out of the provider's published scope list.
    for hidden in ("roles", "service_account"):
        listed = next((x for x in scopes if x["name"] == hidden), None)
        if listed and listed.get("attributes", {}).get("include.in.openid.provider.metadata") != "false":
            listed["attributes"] = dict(listed.get("attributes", {}), **{"include.in.openid.provider.metadata": "false"})
            admin.call("PUT", admin.realm_path(f"/client-scopes/{listed['id']}"), listed)
            done.append(f"stopped advertising the {hidden} scope in the provider metadata")
    components = admin.call("GET", admin.realm_path(f"/components?type={TRUSTED_HOSTS}"))
    policy = next((c for c in components if c["providerId"] == "trusted-hosts" and c.get("subType") == "anonymous"), None)
    if policy is None:
        raise SystemExit("the anonymous Trusted Hosts registration policy is missing from this realm")
    want = {"trusted-hosts": hosts, "host-sending-registration-request-must-match": [str(require_trusted_source).lower()],
            "client-uris-must-match": ["true"]}
    # Keycloak stores multivalued settings as an unordered set, so compare without regard to order.
    same = {k: sorted(v) for k, v in policy["config"].items()} == {k: sorted(v) for k, v in want.items()}
    if not same:
        policy["config"] = want
        admin.call("PUT", admin.realm_path(f"/components/{policy['id']}"), policy)
        done.append("configured the anonymous Trusted Hosts policy")
    # The server asks clients for the plain `openid` scope, which is not a Keycloak client scope, so
    # the anonymous "Allowed Client Scopes" policy would refuse a registration that names it.
    allowed = next((c for c in components if c["providerId"] == "allowed-client-templates" and c.get("subType") == "anonymous"), None)
    if allowed is None:
        raise SystemExit("the anonymous Allowed Client Scopes registration policy is missing from this realm")
    scopes = allowed["config"].get("allowed-client-scopes", [])
    if "openid" not in scopes:
        allowed["config"] = dict(allowed["config"], **{"allowed-client-scopes": sorted(set(scopes) | {"openid"})})
        admin.call("PUT", admin.realm_path(f"/components/{allowed['id']}"), allowed)
        done.append("allowed the openid scope in the anonymous Allowed Client Scopes policy")
    return done


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--keycloak", required=True, help="Keycloak origin, for example https://sso.example")
    parser.add_argument("--realm", default="wk")
    parser.add_argument("--audience", required=True, help="the server's WK_MCP_RESOURCE (also WK_OIDC_AUDIENCE), for example https://knowledge.example")
    parser.add_argument("--trusted-host", action="append", dest="hosts",
                        help="host that registering clients' redirect URIs must be on (repeatable; default localhost and 127.0.0.1)")
    parser.add_argument("--require-trusted-source", action="store_true",
                        help="also require the registration request itself to come from a trusted host (default: off, since developers register from their own machines)")
    parser.add_argument("--ca-file", help="PEM bundle for a Keycloak behind a private CA; replaces the system roots")
    parser.add_argument("--admin-user", default="admin")
    args = parser.parse_args(argv)
    password = os.environ.get("KC_ADMIN_PASSWORD")
    if not password:
        raise SystemExit("set KC_ADMIN_PASSWORD in the environment (it is not accepted as an argument)")
    audience = urllib.parse.urlsplit(args.audience)
    if audience.scheme != "https" or audience.path or args.audience.endswith("/"):
        raise SystemExit("--audience must be the server's https origin with no trailing slash")
    admin = Admin(args.keycloak, args.realm, args.ca_file)
    admin.login(args.admin_user, password)
    done = enable(admin, args.audience, args.hosts or ["localhost", "127.0.0.1"], args.require_trusted_source)
    print("\n".join(done) if done else "nothing to change; the realm is already set up")


if __name__ == "__main__":
    sys.exit(main())
