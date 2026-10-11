#!/usr/bin/env python3
"""Local synthetic fail-closed hook check; never touches a developer repository."""
import argparse
import json
import subprocess
import tempfile
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--agent", type=Path, default=Path("target/debug/wk-agent"))
    args = parser.parse_args()
    agent = args.agent.resolve()
    if not agent.is_file():
        raise SystemExit("Build wk-agent first, or pass --agent /path/to/wk-agent")

    version = "unavailable"
    try:
        result = subprocess.run(
            ["claude", "--version"], capture_output=True, text=True, timeout=3, check=False
        )
        if result.returncode == 0:
            version = result.stdout.strip().split()[0]
    except (OSError, subprocess.TimeoutExpired):
        pass

    with tempfile.TemporaryDirectory(prefix="wk-code-context-live-") as tmp:
        root = Path(tmp)
        state = root / "private-state"
        state.mkdir(mode=0o700)
        repo = root / "synthetic-repo"
        (repo / "backend").mkdir(parents=True)
        target = repo / "backend" / "retry.rs"
        target.write_text("// before confirmation\n", encoding="utf-8")
        before = target.read_bytes()
        status = subprocess.run(
            [str(agent), "--state-dir", str(state), "protect", "status"],
            capture_output=True,
            text=True,
            timeout=5,
            check=True,
        )
        assert json.loads(status.stdout)["status"] == "advisory_only"
        hook = subprocess.run(
            [str(agent), "--state-dir", str(state), "protect-hook", "--client", "claude-code", "--version", "2.1.295"],
            input=json.dumps({
                "hook_event_name": "PreToolUse",
                "tool_name": "Edit",
                "cwd": str(repo),
                "tool_input": {
                    "file_path": str(target),
                    "old_string": "before confirmation",
                    "new_string": "PRIVATE_CANARY after confirmation",
                },
            }),
            capture_output=True,
            text=True,
            timeout=5,
            check=True,
        )
        output = json.loads(hook.stdout)
        assert output["hookSpecificOutput"]["permissionDecision"] == "deny"
        assert target.read_bytes() == before
        assert "PRIVATE_CANARY" not in hook.stdout

    supported = False
    try:
        major, minor, patch = (int(part) for part in version.split(".", 2))
        supported = major > 2 or (major == 2 and (minor > 1 or (minor == 1 and patch >= 295)))
    except (ValueError, AttributeError):
        pass
    print(json.dumps({
        "passed": {
            "missing_enrollment_blocks_edit": True,
            "synthetic_target_unchanged": True,
            "private_input_not_in_hook_response": True,
        },
        "actual_claude_code_version": version,
        "actual_host_edit_acceptance": "unverified" if not supported else "requires_enrolled_synthetic_server_and_manual_ui",
        "evidence_scope": "local synthetic fixture only; does not certify server, browser acknowledgement, or client timeout handling",
    }, sort_keys=True))


if __name__ == "__main__":
    main()
