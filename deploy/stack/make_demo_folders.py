#!/usr/bin/env python3
"""Create one demo project folder per supported coding agent, each with that agent's connection file and standing
instructions exactly as the server's /v1/products/{product}/agent-setup endpoint produces them (the same source the
portal's Connect page and the Admin tab use), so nothing here can drift from what a real developer is given.

  deploy/stack/make-demo-folders.sh [DIR]      # default ~/wk-demo; existing folders are updated in place

Folders: anika-claude-code, anika-claude-desktop, ravi-codex, ravi-cursor, anika-vscode (clones of one tiny shop repo).
No token is written to any file: the files read it from WK_TOKEN (or VS Code asks for it).
"""
import argparse, json, shutil, subprocess, sys, urllib.parse
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import verify  # noqa: E402  (shares the Keycloak sign-in used by the other stack checks)

FOLDERS = [("anika-claude-code", "claude-code"), ("anika-claude-desktop", "claude-desktop"), ("ravi-codex", "codex"),
           ("ravi-cursor", "cursor"), ("anika-vscode", "vscode")]
RETRY = """// Retries a failed payment request up to three times.
pub fn charge_with_retry(order_id: &str, amount_cents: u64) -> Result<(), String> {
    for _attempt in 0..3 {
        if send_charge(order_id, amount_cents).is_ok() {
            return Ok(());
        }
    }
    Err("payment failed after 3 attempts".into())
}
fn send_charge(_order_id: &str, _amount_cents: u64) -> Result<(), String> { Ok(()) }
"""


def git(*args, cwd):
    subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True)


def ensure_origin(origin: Path):
    if (origin / ".git").exists():
        return
    (origin / "backend").mkdir(parents=True, exist_ok=True)
    (origin / "frontend").mkdir(parents=True, exist_ok=True)
    (origin / "backend/retry.rs").write_text(RETRY)
    (origin / "README.md").write_text("# Shop demo\n\nSmall repo for trying Workspace Knowledge with several coding agents.\n")
    (origin / "frontend/price.ts").write_text("export const price = (cents: number) => `$${(cents / 100).toFixed(2)}`;\n")
    git("init", "-q", cwd=origin)
    git("add", "-A", cwd=origin)
    git("-c", "user.name=demo", "-c", "user.email=demo@wk.invalid", "commit", "-qm", "Initial shop demo", cwd=origin)


def write(folder: Path, relative: str, content: str):
    target = folder / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content if content.endswith("\n") else content + "\n")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("dir", nargs="?", default="~/wk-demo")
    p.add_argument("--state-dir", default="~/.local/share/workspace-knowledge/stack")
    p.add_argument("--domain", default="localhost")
    p.add_argument("--kc-port", default="8443")
    p.add_argument("--app-port", default="8444")
    p.add_argument("--product", default="demo")
    p.add_argument("--as-user", default="ravi", help="whose sign-in fetches the setup (any member of the product)")
    p.add_argument("--no-auto-share", action="store_true", help="leave out the 'share finished work automatically' instruction")
    a = p.parse_args()
    root, state = Path(a.dir).expanduser(), Path(a.state_dir).expanduser()
    secrets_ = json.loads((state / "secrets.json").read_text())
    ca, origin = str(state / "certs/ca.pem"), f"https://wk.{a.domain}:{a.app_port}"
    issuer = f"https://sso.{a.domain}:{a.kc_port}/realms/wk"

    token = verify.sign_in(ca, issuer, origin, a.as_user, secrets_["users"][a.as_user])
    query = "" if a.no_auto_share else "?auto_share=true"
    status, text, _ = verify.Browser(ca).call(f"{origin}/v1/products/{a.product}/agent-setup{query}", headers={"Authorization": "Bearer " + token})
    if status != 200:
        sys.exit(f"the server refused the setup request (HTTP {status}); is the stack up and migrated?")
    setup = json.loads(text)
    if not setup["mcp_url"]:
        sys.exit("the server has no MCP address configured (WK_MCP_RESOURCE)")
    clients = {c["id"]: c for c in setup["clients"]}

    root.mkdir(parents=True, exist_ok=True)
    ensure_origin(root / "origin")
    for folder_name, client_id in FOLDERS:
        folder = root / folder_name
        if not folder.exists():
            subprocess.run(["git", "clone", "-q", str(root / "origin"), str(folder)], check=True)
        client = clients[client_id]
        for f in client["files"]:
            if client_id == "claude-desktop":
                # Not project files: a settings entry and a Claude Project's instructions. Keep them beside the repo.
                name = "claude_desktop_config.snippet.json" if f["kind"] == "config" else "claude-project-instructions.md"
                content = f["content"].replace("/absolute/path/to/wk-mcp", shutil.which("wk-mcp") or "/absolute/path/to/wk-mcp")
                write(folder, name, content)
            else:
                write(folder, f["path"], f["content"])
    print(f"Demo folders in {root}, set up from {origin}/v1/products/{a.product}/agent-setup:")
    for folder_name, client_id in FOLDERS:
        print(f"  {folder_name:22} {clients[client_id]['name']}: " + ", ".join(f["path"] if client_id != "claude-desktop" else "(snippet files)" for f in clients[client_id]["files"]))
    print(f"Start each agent from a shell with the right token exported (python3 deploy/stack/stack.py token writes them to {state / 'tokens'}).")


if __name__ == "__main__":
    main()
