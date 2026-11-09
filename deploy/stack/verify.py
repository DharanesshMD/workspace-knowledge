#!/usr/bin/env python3
"""Prove a stack from stack.py works end to end, without a coding agent: sign in as two users through
Keycloak (authorization code + PKCE, like a real client), have the first share a session summary through
the hosted MCP endpoint, and check the second finds it with the first one's name.

  python3 deploy/stack/verify.py [--state-dir DIR] [--domain localhost] [--sharer anika] [--finder ravi] [--tools]
  python3 deploy/stack/verify.py --tokens   # the same, with each user's personal token instead of a Keycloak sign-in
  python3 deploy/stack/verify.py --portal   # sign in to the portal, create, use and revoke a personal token through /v1/tokens
  python3 deploy/stack/verify.py --team-work [--tokens]   # one developer shares work with files; the other asks for it by name, module and file
  python3 deploy/stack/verify.py --admin    # as the admin: add a developer, use the token, reissue, remove (Admin tab's API)
"""
import argparse, base64, hashlib, html, http.cookiejar, json, re, secrets, socket, ssl, sys, urllib.error, urllib.parse, urllib.request
from pathlib import Path

_getaddrinfo = socket.getaddrinfo
socket.getaddrinfo = lambda host, *a, **k: _getaddrinfo("127.0.0.1" if str(host).endswith(".localhost") else host, *a, **k)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


class Browser:
    def __init__(self, ca):
        self.jar = http.cookiejar.CookieJar()
        ctx = ssl.create_default_context(cafile=ca)
        self.open = urllib.request.build_opener(urllib.request.HTTPSHandler(context=ctx), urllib.request.HTTPCookieProcessor(self.jar),
                                                NoRedirect, urllib.request.ProxyHandler({})).open

    def call(self, url, data=None, headers=None, method=None):
        req = urllib.request.Request(url, data=urllib.parse.urlencode(data).encode() if isinstance(data, dict) else data, headers=headers or {}, method=method)
        try:
            with self.open(req, timeout=30) as r:
                return r.status, r.read().decode(), r.headers
        except urllib.error.HTTPError as e:
            return e.code, e.read().decode(), e.headers


def sign_in(ca, issuer, origin, user, password):
    verifier = secrets.token_urlsafe(48)
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    redirect = "http://127.0.0.1:41999/callback"
    b = Browser(ca)
    query = urllib.parse.urlencode({"client_id": "wk-mcp", "response_type": "code", "redirect_uri": redirect, "scope": "openid", "state": "s",
                                    "code_challenge": challenge, "code_challenge_method": "S256", "resource": origin})
    status, page, _ = b.call(f"{issuer}/protocol/openid-connect/auth?{query}")
    action = re.search(r'<form[^>]*id="kc-form-login"[^>]*action="([^"]+)"', page)
    if status != 200 or not action:
        sys.exit(f"{user}: no Keycloak login form (HTTP {status})")
    status, _, headers = b.call(html.unescape(action.group(1)), {"username": user, "password": password, "credentialId": ""})
    location = headers.get("Location", "")
    code = urllib.parse.parse_qs(urllib.parse.urlsplit(location).query).get("code", [None])[0]
    if not code:
        sys.exit(f"{user}: sign-in did not return a code (HTTP {status}); wrong password or the user is not enabled")
    status, body, _ = b.call(f"{issuer}/protocol/openid-connect/token", {"grant_type": "authorization_code", "client_id": "wk-mcp", "code": code,
                                                                          "redirect_uri": redirect, "code_verifier": verifier})
    if status != 200:
        sys.exit(f"{user}: token exchange failed (HTTP {status}) {body[:200]}")
    return json.loads(body)["access_token"]


class Mcp:
    def __init__(self, ca, url, token):
        self.b, self.url, self.token, self.n = Browser(ca), url, token, 0

    def rpc(self, method, params=None):
        self.n += 1
        body = json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params or {}}).encode()
        status, text, _ = self.b.call(self.url, body, {"Authorization": "Bearer " + self.token, "Content-Type": "application/json",
                                                       "Accept": "application/json, text/event-stream"})
        if status != 200:
            sys.exit(f"MCP {method}: HTTP {status} {text[:200]}")
        data = [line[5:].strip() for line in text.splitlines() if line.startswith("data:")] or [text]
        for chunk in data:
            if chunk.startswith("{") and json.loads(chunk).get("id") == self.n:
                return json.loads(chunk)
        sys.exit(f"MCP {method}: no response in {text[:200]}")

    def tool(self, name, arguments):
        reply = self.rpc("tools/call", {"name": name, "arguments": arguments})
        if "error" in reply:
            sys.exit(f"{name}: {reply['error']}")
        if reply["result"].get("isError"):
            sys.exit(f"{name} failed: {json.dumps(reply['result'])[:400]}")
        return reply["result"]


def portal_sign_in(ca, origin, user, password):
    """Sign in to the portal the way a browser does (cookie session). Returns the browser and /v1/me."""
    b, url, page = Browser(ca), f"{origin}/auth/login", ""
    for _ in range(8):
        status, page, headers = b.call(url)
        if status in (301, 302, 303, 307) and headers.get("Location"):
            url = urllib.parse.urljoin(url, headers["Location"])
            continue
        break
    action = re.search(r'<form[^>]*id="kc-form-login"[^>]*action="([^"]+)"', page)
    if not action:
        sys.exit(f"{user}: the portal did not lead to a Keycloak login form (HTTP {status})")
    status, _, headers = b.call(html.unescape(action.group(1)), {"username": user, "password": password, "credentialId": ""})
    callback = headers.get("Location", "")
    if not callback.startswith(f"{origin}/auth/callback"):
        sys.exit(f"{user}: portal sign-in did not return to the callback (HTTP {status})")
    b.call(callback)
    status, body, _ = b.call(f"{origin}/v1/me")
    if status != 200:
        sys.exit(f"{user}: no portal session after sign-in (HTTP {status})")
    return b, json.loads(body)


def mcp_status(ca, url, token):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {},
                                                                                        "clientInfo": {"name": "verify", "version": "0"}}}).encode()
    return Browser(ca).call(url, body, {"Authorization": "Bearer " + token, "Content-Type": "application/json",
                                        "Accept": "application/json, text/event-stream"})[0]


def portal_check(ca, origin, product, user, password):
    b, me = portal_sign_in(ca, origin, user, password)
    limits = me.get("personal_tokens")
    if not limits:
        sys.exit("FAIL: the server has personal tokens switched off (run stack.py up without --no-personal-tokens)")
    print(f"ok  {user} signed in to the portal; personal tokens allowed up to {limits['max_days']} days")
    api = {"Origin": origin, "X-CSRF-Token": me["csrf_token"], "Content-Type": "application/json"}
    created = json.dumps({"product": product, "label": "verify", "days": 1}).encode()
    status, _, _ = b.call(f"{origin}/v1/tokens", created, {k: v for k, v in api.items() if k != "X-CSRF-Token"})
    assert status == 403, f"creating a token without the CSRF token should be refused, got HTTP {status}"
    print("ok  creating a token without the CSRF token is refused (403)")
    status, body, headers = b.call(f"{origin}/v1/tokens", created, api)
    made = json.loads(body)
    assert status == 201 and re.fullmatch(r"wkpt_[0-9a-f]{64}", made.get("token", "")), f"create failed: HTTP {status} {body[:200]}"
    assert headers.get("Cache-Control") == "no-store", "the token response must not be cached"
    token, url = made["token"], f"{origin}/mcp/{product}"
    print("ok  portal created a token (shown once, no-store)")
    listed = b.call(f"{origin}/v1/tokens")[1]
    assert made["id"] in listed and token not in listed, "the list must show the token's metadata and never its secret"
    print("ok  the token list shows it without the secret")
    assert mcp_status(ca, url, token) == 200, "the token should open the MCP endpoint"
    print(f"ok  the token opens {url}")
    for path in ("/v1/me", f"/v1/policy?product={product}"):
        status = Browser(ca).call(origin + path, headers={"Authorization": "Bearer " + token})[0]
        assert status == 401, f"{path} must reject a personal token, got HTTP {status}"
    print("ok  the same token is rejected on the REST API (401)")
    assert mcp_status(ca, f"{origin}/mcp/other-product", token) in (401, 403), "the token must not open another product"
    print("ok  and on another product's MCP address")
    assert Browser(ca).call(f"{origin}/v1/tokens", headers={"Authorization": "Bearer " + token})[0] == 401, "a token must not manage tokens"
    print("ok  a token cannot list or mint tokens")
    status = b.call(f"{origin}/v1/tokens/{made['id']}", headers=api, method="DELETE")[0]
    assert status == 200, f"revoke failed: HTTP {status}"
    assert mcp_status(ca, url, token) == 401, "a revoked token must stop working at once"
    print("ok  revoking it stops it working at once (401)")
    print("PASS: portal token lifecycle works end to end.")


def admin_check(ca, origin, issuer, product, admin_user, passwords, other_user):
    b, me = portal_sign_in(ca, origin, admin_user, passwords[admin_user])
    mine = next((p for p in me["products"] if p["product"] == product), {})
    assert mine.get("is_admin") is True, f"{admin_user} is not an admin of {product} (run stack.py up, which creates the admin)"
    assert me.get("personal_tokens"), "personal tokens are switched off on the server"
    print(f"ok  {admin_user} signed in to the portal and /v1/me says is_admin")
    api = {"Origin": origin, "X-CSRF-Token": me["csrf_token"], "Content-Type": "application/json"}
    base = f"{origin}/v1/admin/products/{product}/users"
    mcp_url = f"{origin}/mcp/{product}"
    name = "verify-" + secrets.token_hex(3)

    def post(path, body):
        status, text, headers = b.call(path, json.dumps(body).encode(), api)
        return status, (json.loads(text) if text[:1] == "{" else {}), headers
    status, text, _ = b.call(base)
    listing = json.loads(text)
    assert status == 200 and listing["services"] and listing["mcp_url"] == mcp_url, f"list failed: HTTP {status} {text[:200]}"
    services = listing["services"][:1]
    print(f"ok  the Admin list works; the admin may grant {', '.join(listing['services'])}")

    status, made, headers = post(base, {"user": name, "services": services, "can_review": False, "days": 1})
    assert status == 201 and re.fullmatch(r"wkpt_[0-9a-f]{64}", made.get("token", "")), f"create failed: HTTP {status} {made}"
    assert made["mcp_url"] == mcp_url and headers.get("Cache-Control") == "no-store"
    token = made["token"]
    print(f"ok  created {name}: token and MCP URL returned once (no-store)")
    status, text, _ = b.call(base)
    row = next(u for u in json.loads(text)["users"] if u["user"] == name)
    assert row["token_only"] and len(row["tokens"]) == 1 and token not in text, "the list must show the new account without any secret"
    print("ok  the list shows the new account as token-only, with token metadata and no secret")

    assert mcp_status(ca, mcp_url, token) == 200, "the new developer's token should open the MCP endpoint"
    reply = Mcp(ca, mcp_url, token)
    reply.rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "verify", "version": "0"}})
    reply.tool("knowledge.search", {"query": "anything"})
    print(f"ok  {name}'s token works on the MCP address, as {name}")
    assert Browser(ca).call(f"{origin}/v1/me", headers={"Authorization": "Bearer " + token})[0] == 401
    assert Browser(ca).call(base, headers={"Authorization": "Bearer " + token})[0] == 401, "a token must not reach the Admin API"
    print("ok  the token is refused on the REST API and the Admin API")

    assert post(base, {"user": name, "services": services, "days": 1})[0] == 409
    assert post(base, {"user": other_user, "services": services, "days": 1})[0] == 409, "an existing signed-in developer's name must be refused"
    assert post(base, {"user": "Bad Name", "services": services, "days": 1})[0] == 422
    assert post(base, {"user": name + "x", "services": ["not-a-service"], "days": 1})[0] == 403
    assert post(base, {"user": name + "y", "services": services, "days": 9999})[0] == 422
    print("ok  a taken name, another developer's name, a bad name, a service outside the admin's scope and a too-long lifetime are all refused")

    status, again, _ = post(f"{base}/{name}/token", {"days": 1})
    assert status == 200 and again["token"] != token and mcp_status(ca, mcp_url, again["token"]) == 200
    assert mcp_status(ca, mcp_url, token) == 401, "reissuing must revoke the old token"
    status, _, _ = post(f"{base}/{other_user}/token", {"days": 1})
    assert status == 403, "an admin must not mint a token for a signed-in developer"
    print("ok  reissue replaces the token (old one 401); minting for a signed-in developer is refused")

    rb, rme = portal_sign_in(ca, origin, other_user, passwords[other_user])
    assert next(p for p in rme["products"] if p["product"] == product)["is_admin"] is False
    rapi = {"Origin": origin, "X-CSRF-Token": rme["csrf_token"], "Content-Type": "application/json"}
    assert rb.call(base)[0] == 403
    assert rb.call(base, json.dumps({"user": "sneaky", "services": services, "days": 1}).encode(), rapi)[0] == 403
    print(f"ok  {other_user} (not an admin) cannot list or create")

    assert b.call(f"{base}/{name}", headers=api, method="DELETE")[0] == 200
    assert mcp_status(ca, mcp_url, again["token"]) == 401, "removal must revoke the token at once"
    status, _, _ = post(base, {"user": name, "services": services, "days": 1})
    assert status == 409, "a retired principal name must not be reassigned"
    assert mcp_status(ca, mcp_url, again["token"]) == 401 and mcp_status(ca, mcp_url, token) == 401
    print("ok  removal revokes tokens and permanently reserves the principal name")
    print("PASS: the admin can add, hand over, reissue and remove developers end to end.")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--state-dir", default="~/.local/share/workspace-knowledge/stack")
    p.add_argument("--domain", default="localhost")
    p.add_argument("--kc-port", default="8443")
    p.add_argument("--app-port", default="8444")
    p.add_argument("--product", default="demo")
    p.add_argument("--sharer", default="anika")
    p.add_argument("--finder", default="ravi")
    p.add_argument("--keep", action="store_true", help="leave the test card published instead of withdrawing it")
    p.add_argument("--withdraw", action="append", default=[], metavar="CARD_ID", help="withdraw an earlier test card (as the sharer) and stop")
    p.add_argument("--tokens", action="store_true", help="authenticate the MCP calls with each user's personal token (from `stack.py token`)")
    p.add_argument("--portal", action="store_true", help="check the portal's token create/list/revoke routes as the sharer, then stop")
    p.add_argument("--team-work", action="store_true", help="check knowledge.team_work: name, module, file, time window, unknown name")
    p.add_argument("--admin", action="store_true", help="check the Admin tab's API as the admin user, then stop")
    p.add_argument("--admin-user", default="owner", help="the portal admin (created by stack.py up)")
    p.add_argument("--tools", action="store_true", help="print the tool list and schemas, then stop")
    a = p.parse_args()
    state = Path(a.state_dir).expanduser()
    secrets_ = json.loads((state / "secrets.json").read_text())
    ca = str(state / "certs/ca.pem")
    origin = f"https://wk.{a.domain}:{a.app_port}"
    issuer = f"https://sso.{a.domain}:{a.kc_port}/realms/wk"
    if a.admin:
        return admin_check(ca, origin, issuer, a.product, a.admin_user, secrets_["users"], a.finder)
    if a.portal:
        return portal_check(ca, origin, a.product, a.sharer, secrets_["users"][a.sharer])
    mcp = {}
    for user in (a.sharer, a.finder):
        if a.tokens:
            token_file = state / "tokens" / f"{user}.token"
            if not token_file.exists():
                sys.exit(f"no token for {user}: run `python3 deploy/stack/stack.py token` first")
            mcp[user] = Mcp(ca, f"{origin}/mcp/{a.product}", token_file.read_text().strip())
        else:
            mcp[user] = Mcp(ca, f"{origin}/mcp/{a.product}", sign_in(ca, issuer, origin, user, secrets_["users"][user]))
        mcp[user].rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "verify", "version": "0"}})
        print(f"ok  {user} {'used a personal token' if a.tokens else 'signed in through Keycloak'} and the MCP endpoint accepted it")
    def withdraw(card_id):
        # Withdrawing is a REST call, which a personal token deliberately cannot make: clean up as the signed-in user.
        owner_token = sign_in(ca, issuer, origin, a.sharer, secrets_["users"][a.sharer]) if a.tokens else mcp[a.sharer].token
        status, text, _ = mcp[a.sharer].b.call(f"{origin}/v1/cards/{card_id}/withdraw",
                                              json.dumps({"product": a.product, "revision": 1}).encode(),
                                              {"Authorization": "Bearer " + owner_token, "Content-Type": "application/json"})
        print(f"{'ok ' if status < 300 else 'FAIL'} withdraw {card_id}: HTTP {status}")
    if a.withdraw:
        return [withdraw(c) for c in a.withdraw] and None
    if a.tools:
        print(json.dumps(mcp[a.sharer].rpc("tools/list")["result"]["tools"], indent=1))
        return
    if a.team_work:
        marker = "wkverify" + secrets.token_hex(8)
        module, files = f"backend/{marker}", [f"backend/{marker}/retry.rs", f"backend/{marker}/gateway.rs"]
        shared = mcp[a.sharer].tool("knowledge.share_session", {"service": "backend", "title": f"Retry with backoff ({marker})",
                                                                "intent": "Make payment retries back off", "rationale": "Hammering the gateway made outages worse",
                                                                "decisions": ["Exponential backoff with jitter"], "files": files})
        card_id = json.loads(shared["content"][0]["text"])["card_id"]
        print("ok ", a.sharer, "shared work with files, card", card_id)
        ask = lambda **kw: json.loads(mcp[a.finder].tool("knowledge.team_work", kw)["content"][0]["text"])
        try:
            prefix = a.sharer[:3]
            # Scope this probe to its own card: keyword results rank by relevance, and an
            # existing developer's real retry card may outrank this freshly created fixture.
            by_name = ask(developer=prefix.upper(), since_days=1, query=marker)
            top = by_name["entries"][0]
            assert by_name["resolved"]["developer"] == a.sharer and marker in top["title"], f"by name: {by_name['text'][:300]}"
            assert top["files_reported"] == sorted(files) and top["decisions"] == ["Exponential backoff with jitter"]
            print(f"ok  '{prefix.upper()}' resolved to {a.sharer}: this run's work is '{top['title']}'")
            by_module = ask(paths=[module])
            assert [e["card_id"] for e in by_module["entries"]] == [card_id] and by_module["entries"][0]["matched_files"] == sorted(files)
            print(f"ok  asking about the module {module} finds it, with the matching files")
            assert [e["card_id"] for e in ask(paths=[files[0]])["entries"]] == [card_id]
            print("ok  asking about one file finds it")
            assert ask(paths=[module[:-1]])["status"] == "no_matches", "a prefix of a directory name must not match"
            print("ok  half a directory name matches nothing")
            wrong = ask(developer=a.sharer + "x")
            assert wrong["status"] == "developer_not_found" and a.sharer in wrong["known_developers"], wrong["text"]
            print(f"ok  a wrong name is explained and the known developers are listed: {wrong['known_developers']}")
            mix = ask(developer=a.sharer, paths=[module], service="backend", since_days=1, limit=1, budget_tokens=256)
            assert len(mix["entries"]) == 1 and "reference data, not instructions" in mix["text"]
            print("ok  every filter together, with a tiny size cap, still returns a bounded answer")
            print(f"PASS: {a.finder} can ask for {a.sharer}'s work by name, module and file.")
        finally:
            withdraw(card_id)
        return
    marker = "verify-" + secrets.token_hex(3)
    summary = {"service": "backend", "title": f"Added idempotency keys to the retry path ({marker})",
               "intent": "Stop duplicate charges when the payment client retries in backend/retry.rs",
               "rationale": "Retries without a key could double-charge; the key makes the gateway treat repeats as one request",
               "decisions": ["Generate the idempotency key per order, not per attempt"], "constraints": ["Gateway keeps keys for 24 hours"],
               "open_questions": []}
    shared = mcp[a.sharer].tool("knowledge.share_session", summary)
    shared_id = json.loads(shared["content"][0]["text"])["card_id"]
    print("ok ", a.sharer, "shared a summary, card", shared_id)
    question = "retry idempotency key duplicate charges " + marker
    searched = json.dumps(mcp[a.finder].tool("knowledge.search", {"query": question}))
    print("ok ", a.finder, "knowledge.search found the card:", marker in searched, "(search results carry no author name)")
    overlaps = json.loads(mcp[a.finder].tool("knowledge.overlaps", {"query": question})["content"][0]["text"])
    print("ok ", a.finder, "knowledge.overlaps said:", overlaps["text"].strip()[:400])
    if marker not in searched or not any(n["developer"] == a.sharer and marker in n["summary"] for n in overlaps["notices"]):
        sys.exit(f"FAIL: {a.finder} was not told that {a.sharer} already did this")
    if not a.keep:
        withdraw(shared_id)
    print(f"PASS: {a.finder}'s agent is told that {a.sharer} already did this" + (" (personal tokens only)." if a.tokens else "."))


if __name__ == "__main__":
    main()
