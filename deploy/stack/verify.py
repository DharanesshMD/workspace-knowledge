#!/usr/bin/env python3
"""Prove a stack from stack.py works end to end, without a coding agent: sign in as two users through
Keycloak (authorization code + PKCE, like a real client), have the first share a session summary through
the hosted MCP endpoint, and check the second finds it with the first one's name.

  python3 deploy/stack/verify.py [--state-dir DIR] [--domain localhost] [--sharer anika] [--finder ravi] [--tools]
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

    def call(self, url, data=None, headers=None):
        req = urllib.request.Request(url, data=urllib.parse.urlencode(data).encode() if isinstance(data, dict) else data, headers=headers or {})
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
    p.add_argument("--tools", action="store_true", help="print the tool list and schemas, then stop")
    a = p.parse_args()
    state = Path(a.state_dir).expanduser()
    secrets_ = json.loads((state / "secrets.json").read_text())
    ca = str(state / "certs/ca.pem")
    origin = f"https://wk.{a.domain}:{a.app_port}"
    issuer = f"https://sso.{a.domain}:{a.kc_port}/realms/wk"
    mcp = {}
    for user in (a.sharer, a.finder):
        mcp[user] = Mcp(ca, f"{origin}/mcp/{a.product}", sign_in(ca, issuer, origin, user, secrets_["users"][user]))
        mcp[user].rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "verify", "version": "0"}})
        print(f"ok  {user} signed in through Keycloak and the MCP endpoint accepted the token")
    def withdraw(card_id):
        status, text, _ = mcp[a.sharer].b.call(f"{origin}/v1/cards/{card_id}/withdraw",
                                              json.dumps({"product": a.product, "revision": 1}).encode(),
                                              {"Authorization": "Bearer " + mcp[a.sharer].token, "Content-Type": "application/json"})
        print(f"{'ok ' if status < 300 else 'FAIL'} withdraw {card_id}: HTTP {status}")
    if a.withdraw:
        return [withdraw(c) for c in a.withdraw] and None
    if a.tools:
        print(json.dumps(mcp[a.sharer].rpc("tools/list")["result"]["tools"], indent=1))
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
    print(f"PASS: {a.finder}'s agent is told that {a.sharer} already did this.")


if __name__ == "__main__":
    main()
