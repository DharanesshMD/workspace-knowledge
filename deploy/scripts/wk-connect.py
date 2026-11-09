#!/usr/bin/env python3
"""Configure installed Codex/Antigravity clients without putting a token in IDE settings."""
import argparse
import getpass
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import stat
import subprocess
import sys
import tempfile
try:
    import tomllib
except ImportError:
    print("wk-connect requires Python 3.11 or newer", file=sys.stderr)
    sys.exit(1)


def read_file(path, maximum=1048576, private=False):
    fd = os.open(str(path), os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as file:
        info = os.fstat(file.fileno())
        if not stat.S_ISREG(info.st_mode) or (private and info.st_mode & 0o077):
            raise ValueError("expected a private regular file")
        content = file.read(maximum + 1)
    if len(content) > maximum:
        raise ValueError("file exceeds size limit")
    return content


def directory(path, home, private=False):
    if path == home or path == path.parent:
        return
    directory(path.parent, home)
    if path.is_symlink():
        raise ValueError("refusing a symlinked configuration directory")
    path.mkdir(mode=0o700, exist_ok=True)
    if not path.is_dir() or (private and path.stat().st_mode & 0o077):
        raise ValueError("managed state directory must be private (mode 0700)")


def write_file(path, content):
    if path.is_symlink():
        raise ValueError("refusing a symlinked configuration file")
    fd, temporary = tempfile.mkstemp(prefix=".wk-", dir=str(path.parent))
    try:
        with os.fdopen(fd, "wb") as file:
            file.write(content)
            file.flush()
            os.fsync(file.fileno())
        os.replace(temporary, str(path))
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def encoded(value):
    return (json.dumps(value, indent=2, ensure_ascii=False) + "\n").encode()


def command(args, env=None):
    result = subprocess.run(args, env=env, capture_output=True, text=True, timeout=30)
    if result.returncode:
        raise ValueError("client configuration command failed")
    return result.stdout


def check_connection(entry):
    requests = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-03-26", "capabilities": {},
            "clientInfo": {"name": "wk-connect", "version": "1"}}},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
    ]
    env = dict(os.environ)
    for key in ("WK_TOKEN", "WK_MCP_URL", "WK_SERVER_CA_FILE"):
        env.pop(key, None)
    result = subprocess.run([entry["command"]] + entry["args"], env=env,
                            input="".join(json.dumps(r) + "\n" for r in requests),
                            capture_output=True, text=True, timeout=30)
    if result.returncode:
        raise ValueError("MCP connection failed; check the URL, token-file permissions and CA bundle")
    replies = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
    if not all(isinstance(reply, dict) for reply in replies):
        raise ValueError("invalid MCP connection response")
    for reply in replies:
        if "error" in reply:
            text = str(reply["error"].get("message", ""))
            if "HTTP 401" in text:
                raise ValueError("server refused the token (expired, revoked or invalid)")
            raise ValueError("MCP connection failed; check server access, network and certificates")
    tools = next((r.get("result", {}).get("tools") for r in replies if r.get("id") == 2), None)
    if not isinstance(tools, list) or not any(isinstance(t, dict) and t.get("name") == "knowledge.team_work" for t in tools):
        raise ValueError("the endpoint did not provide Workspace Knowledge tools")
    return len(tools)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["connect", "check", "disconnect"])
    parser.add_argument("--client", choices=["codex", "antigravity"], required=True)
    parser.add_argument("--name", default="workspace-knowledge")
    parser.add_argument("--url", help="HTTPS product MCP address, for example https://knowledge.example/mcp/product")
    token = parser.add_mutually_exclusive_group()
    token.add_argument("--token-stdin", action="store_true", help="read a token from standard input, never an argument")
    token.add_argument("--token-file", type=Path, help="copy a private token file into managed user state")
    parser.add_argument("--ca-file", type=Path, help="private-CA certificate bundle; no system trust is changed")
    parser.add_argument("--binary", type=Path, help="installed wk-mcp (defaults to the sibling binary or PATH)")
    parser.add_argument("--codex", help="Codex CLI path when it is not on PATH")
    parser.add_argument("--replace", action="store_true", help="back up and replace an existing unowned server with this name")
    args = parser.parse_args()
    if os.name != "posix":
        raise ValueError("this onboarding helper currently supports macOS and Linux")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}", args.name):
        raise ValueError("name must contain only letters, digits, underscores and hyphens")
    raw_home = Path(os.environ["HOME"]).absolute()
    home = raw_home.resolve()
    state = home / ".local/share/workspace-knowledge/clients" / args.client / args.name
    codex_home = Path(os.environ.get("CODEX_HOME", str(home / ".codex"))).expanduser().absolute()
    if codex_home.is_relative_to(raw_home):
        codex_home = home / codex_home.relative_to(raw_home)
    config = codex_home / "config.toml" if args.client == "codex" else home / ".gemini/config/mcp_config.json"
    directory(config.parent, home)
    if config.is_symlink():
        raise ValueError("refusing a symlinked configuration file")
    manifest = state / "connection.json"
    previous = json.loads(read_file(manifest, private=True)) if manifest.exists() else None

    def credential_directories():
        # Include the original pre-endpoint layout for existing installations.
        yield state
        if state.exists():
            for folder in state.iterdir():
                if re.fullmatch(r"endpoint-[0-9a-f]{64}", folder.name):
                    if folder.is_symlink() or not folder.is_dir():
                        raise ValueError("invalid managed credential directory")
                    yield folder
    cli_env = dict(os.environ, CODEX_HOME=str(config.parent))
    codex = None
    if args.client == "codex":
        bundled = Path("/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex")
        codex = args.codex or shutil.which("codex") or (str(bundled) if bundled.is_file() else None)
        if not codex:
            raise ValueError("Codex CLI is not installed; supply --codex PATH")

    def config_data(content):
        if args.client == "antigravity":
            data = json.loads(content) if content is not None else {}
            if not isinstance(data, dict) or not isinstance(data.get("mcpServers", {}), dict):
                raise ValueError("invalid Antigravity MCP configuration")
        else:
            data = tomllib.loads(content.decode()) if content is not None else {}
            if not isinstance(data.get("mcp_servers", {}), dict):
                raise ValueError("invalid Codex MCP configuration")
        return data

    server_key = "mcp_servers" if args.client == "codex" else "mcpServers"

    def config_bytes():
        return read_file(config) if config.exists() else None

    def current_entry():
        return config_data(config_bytes()).get(server_key, {}).get(args.name)

    def set_entry(entry):
        before = config_bytes()
        data = config_data(before)
        if data.get(server_key, {}).get(args.name) != current:
            raise ValueError("IDE entry changed during setup; review it and retry")
        if args.client == "codex":
            # The native CLI may discard fields it does not expose in `mcp get`.
            # Stage it privately, then verify every unrelated setting before committing.
            with tempfile.TemporaryDirectory(prefix="wk-codex-config-") as temporary:
                staged = Path(temporary) / "config.toml"
                write_file(staged, before or b"")
                command(([codex, "mcp", "remove", args.name] if entry is None else
                         [codex, "mcp", "add", args.name, "--env", "WK_SERVER_CA_FILE=", "--",
                          entry["command"]] + entry["args"]),
                        env=dict(cli_env, CODEX_HOME=temporary))
                after = read_file(staged)
            def unrelated(content):
                data = tomllib.loads(content.decode())
                servers = data.get("mcp_servers", {})
                servers.pop(args.name, None)
                if not servers:
                    data.pop("mcp_servers", None)
                return data
            if unrelated(before or b"") != unrelated(after):
                raise ValueError("Codex CLI would change unrelated settings; update the CLI or configure it manually")
        else:
            servers = data.setdefault("mcpServers", {})
            if entry is None:
                servers.pop(args.name, None)
            else:
                servers[args.name] = entry
            after = encoded(data)
        if config_bytes() != before:
            raise ValueError("IDE settings changed during setup; review them and retry")
        write_file(config, after)
        return before, after

    current = current_entry()
    if args.action != "connect":
        if not previous:
            if args.action == "disconnect" and current is None:
                print(json.dumps({"disconnected": True, "client": args.client, "already_disconnected": True}))
                return
            raise ValueError("no managed connection for this client/name")
        if current != previous["entry"] and not (args.action == "disconnect" and current is None):
            raise ValueError("IDE entry was changed outside wk-connect; refusing to overwrite it")
        if args.action == "check":
            count = check_connection(current)
            print(json.dumps({"connected": True, "client": args.client, "tools": count, "url": previous["url"]}))
        else:
            folders = list(credential_directories())
            if current is not None:
                set_entry(None)
            for folder in folders:
                for name in ("token", "ca.pem"):
                    (folder / name).unlink(missing_ok=True)
            manifest.unlink(missing_ok=True)
            print(json.dumps({"disconnected": True, "client": args.client, "other_servers_preserved": True}))
        return
    if not args.url:
        raise ValueError("connect requires --url")
    if current is not None and (not previous or current != previous["entry"]) and not args.replace:
        raise ValueError("a different server already uses this name; choose --name or explicitly use --replace")
    sibling = Path(__file__).resolve().parent / "wk-mcp"
    binary = args.binary or (sibling if sibling.is_file() else shutil.which("wk-mcp"))
    if not binary or not Path(binary).is_file():
        raise ValueError("install wk-mcp first, or supply --binary PATH")
    binary = str(Path(binary).resolve())
    if args.token_file:
        secret = read_file(args.token_file, maximum=514, private=True).decode().strip()
    elif args.token_stdin:
        raw = sys.stdin.read(515)
        if len(raw) > 514:
            raise ValueError("token input exceeds size limit")
        secret = raw.strip()
    else:
        secret = getpass.getpass("Personal MCP token (input hidden): ").strip()
    if not secret or len(secret) > 512 or not secret.isascii() or any(c.isspace() or ord(c) < 33 or ord(c) > 126 for c in secret):
        raise ValueError("invalid token; it must be a single printable token")
    ca = read_file(args.ca_file, maximum=65536) if args.ca_file else None
    directory(state, home, private=True)
    credentials = state / ("endpoint-" + hashlib.sha256(args.url.encode()).hexdigest())
    def entry_at(folder):
        options = ["proxy", "--url", args.url, "--token-file", str(folder / "token")]
        if ca is not None:
            options += ["--ca-file", str(folder / "ca.pem")]
        return {"command": binary, "args": options, "env": {"WK_SERVER_CA_FILE": ""}}
    with tempfile.TemporaryDirectory(prefix=".check-", dir=str(state)) as temporary:
        stage = Path(temporary)
        write_file(stage / "token", (secret + "\n").encode())
        if ca is not None:
            write_file(stage / "ca.pem", ca)
        count = check_connection(entry_at(stage))
    directory(credentials, home, private=True)
    backup = state / "original-config.backup"
    if config.exists() and not backup.exists():
        write_file(backup, read_file(config))
    paths = [credentials / "token", credentials / "ca.pem", manifest]
    old = {p: read_file(p, private=True) if p.exists() else None for p in paths}
    entry = entry_at(credentials)
    if current_entry() != current:
        raise ValueError("IDE entry changed during setup; retry after reviewing it")
    before_config = read_file(config) if config.exists() else None
    if args.replace and before_config is not None:
        write_file(state / ("replaced-config-" + secrets.token_hex(8) + ".backup"), before_config)
    committed_config = None
    try:
        write_file(paths[0], (secret + "\n").encode())
        if ca is not None:
            write_file(paths[1], ca)
        if current != entry:
            committed_config = set_entry(entry)
        write_file(manifest, encoded({"url": args.url, "entry": entry}))
    except Exception:
        if committed_config is not None and config_bytes() == committed_config[1]:
            if committed_config[0] is None:
                config.unlink(missing_ok=True)
            else:
                write_file(config, committed_config[0])
        for path, content in old.items():
            if content is None:
                path.unlink(missing_ok=True)
            else:
                write_file(path, content)
        raise
    print(json.dumps({"connected": True, "client": args.client, "tools": count,
                      "url": args.url, "config": str(config), "token_in_ide_config": False,
                      "next": "Start a new client session; sharing remains on explicit request."}))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.SubprocessError):
        # Never print subprocess output, file contents, or a token passed on stdin.
        error = sys.exc_info()[1]
        message = str(error) if isinstance(error, ValueError) and not isinstance(error, json.JSONDecodeError) else "configuration or connection unavailable; existing credentials were not printed"
        print("wk-connect: " + message, file=sys.stderr)
        sys.exit(1)
