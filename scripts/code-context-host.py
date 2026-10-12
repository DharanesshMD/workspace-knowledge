"""Check an explicit Claude executable against an owned loopback model fixture.

Uses no real provider account or global client profile. This verifies native
hook denial/crash/timeout behavior, not browser acknowledgement or production.
"""
import argparse
from claude_fixture import ModelFixture
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
if os.environ.get('WK_HOST_FIXTURE_TRACE') == '1':
    import faulthandler
    faulthandler.dump_traceback_later(10)
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--claude', type=Path, required=True, help='Explicit supported Claude executable; never installed or upgraded by this harness')
parser.add_argument('--agent', type=Path, default=Path('target/debug/wk-agent'))
parser.add_argument('--tool', choices=['Edit', 'Write', 'MultiEdit'], default='Edit')
args = parser.parse_args()
CLAUDE = args.claude.resolve()
AGENT = args.agent.resolve()
if not CLAUDE.is_file() or not AGENT.is_file():
    parser.error('Both executables must already exist')
VERSION = subprocess.check_output([str(CLAUDE), '--version'], text=True, timeout=10).split()[0]
try:
    parts = tuple((int(value) for value in VERSION.split('.')))
    supported = len(parts) == 3 and parts >= (2, 1, 295)
except ValueError:
    supported = False
if not supported:
    parser.error('Claude Code 2.1.295 or later is required; no host edit flow was started')
results = []
for case in ['baseline', 'missing_enrollment', 'hook_crash', 'hook_timeout']:
    with tempfile.TemporaryDirectory(prefix='wk-v01-native-host-') as tmp:
        root = Path(tmp)
        repo = root / 'repo'
        repo.mkdir()
        state = root / 'private'
        state.mkdir(mode=448)
        target = repo / 'retry.txt'
        target.write_text('before confirmation\n')
        original_bytes = target.read_bytes()

        server = ModelFixture(target, args.tool).start()
        hook = state / 'hook.py'
        marker = state / 'hook-invoked'
        if case == 'missing_enrollment':
            hook.write_text('import pathlib,subprocess,sys\npathlib.Path(' + repr(str(marker)) + ').write_text("invoked")\nr=subprocess.run(' + repr([str(AGENT), '--state-dir', str(state), 'protect-hook', '--client', 'claude-code', '--version', VERSION]) + ',input=sys.stdin.buffer.read(),capture_output=True)\nsys.stdout.buffer.write(r.stdout)\nsys.exit(r.returncode)\n')
        elif case == 'hook_crash':
            hook.write_text('import pathlib,sys\npathlib.Path(' + repr(str(marker)) + ').write_text("invoked")\nsys.exit(1)\n')
        elif case == 'hook_timeout':
            hook.write_text('import pathlib,time\npathlib.Path(' + repr(str(marker)) + ').write_text("invoked")\ntime.sleep(10)\n')
        settings = state / 'settings.json'
        config = {} if case == 'baseline' else {'hooks': {'PreToolUse': [{'matcher': args.tool, 'hooks': [{'type': 'command', 'command': shlex.quote(sys.executable) + ' ' + shlex.quote(str(hook)), 'onFailure': 'block', 'timeout': 2}]}]}}
        settings.write_text(json.dumps(config))
        mcp = state / 'mcp.json'
        mcp.write_text('{"mcpServers":{}}')
        env = dict(os.environ)
        for key in list(env):
            if key.startswith(('ANTHROPIC_', 'CLAUDE_')):
                env.pop(key)
        env.update(ANTHROPIC_API_KEY='test', ANTHROPIC_BASE_URL='http://127.0.0.1:' + str(server.server_port), CLAUDE_CONFIG_DIR=str(state / 'claude'), CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1', DISABLE_TELEMETRY='1', DISABLE_ERROR_REPORTING='1', DISABLE_AUTOUPDATER='1')
        try:
            result = subprocess.run([str(CLAUDE), '-p', 'Use Read then ' + args.tool + ' to replace before confirmation with after confirmation in ' + str(target) + '.', '--restricted', '--setting-sources', '', '--settings', str(settings), '--strict-mcp-config', '--mcp-config', str(mcp), '--no-session-persistence', '--tools', 'Read,' + args.tool, '--allowedTools', 'Read,' + args.tool, '--permission-mode', 'acceptEdits', '--output-format', 'json', '--max-turns', '4'], cwd=repo, env=env, capture_output=True, text=True, timeout=60)
        finally:
            server.stop()
        final_bytes = target.read_bytes()
        changed = final_bytes != original_bytes
        expected_bytes = b'after confirmation\n' if case == 'baseline' else original_bytes
        record = {'case': case, 'tool': args.tool, 'offered_tools': sorted(server.offered_tools), 'claude_version': VERSION, 'exit_code': result.returncode, 'model_requests': server.requests, 'hook_invoked': marker.exists(), 'target_changed': changed, 'expected_unchanged': case != 'baseline'}
        results.append(record)
        print(json.dumps(record), flush=True)
        if result.returncode or server.requests < 3 or final_bytes != expected_bytes or (case != 'baseline' and (not marker.exists())):
            print('Fixture failed; client output is intentionally withheld.', flush=True)
            raise SystemExit(1)
print(json.dumps({'passed': True, 'scope': 'actual Claude host with synthetic loopback model only; no cloud, real credentials or browser acknowledgement', 'cases': results}))
