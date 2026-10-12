"""Check an explicit Claude executable against an owned loopback model fixture.

Uses no real provider account or global client profile. This verifies native
hook denial/crash/timeout behavior, not browser acknowledgement or production.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import threading
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--claude', type=Path, required=True, help='Explicit supported Claude executable; never installed or upgraded by this harness')
parser.add_argument('--agent', type=Path, default=Path('target/debug/wk-agent'))
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
        count = [0]

        class Handler(http.server.BaseHTTPRequestHandler):

            def log_message(self, *args):
                pass

            def do_GET(self):
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(b'{}')

            def do_POST(self):
                length = int(self.headers.get('Content-Length', 0))
                assert length < 1048576
                self.rfile.read(length)
                if '/messages' not in self.path:
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/json')
                    self.end_headers()
                    self.wfile.write(b'{}')
                    return
                count[0] += 1
                index = count[0]
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.send_header('Connection', 'close')
                self.end_headers()

                def send(kind, data):
                    self.wfile.write(('event: ' + kind + '\ndata: ' + json.dumps(data) + '\n\n').encode())
                    self.wfile.flush()
                send('message_start', {'type': 'message_start', 'message': {'id': 'msg_wk_fixture_' + str(index), 'type': 'message', 'role': 'assistant', 'content': [], 'model': 'claude-sonnet-4-6', 'stop_reason': None, 'stop_sequence': None, 'usage': {'input_tokens': 10, 'output_tokens': 0}}})
                if index <= 2:
                    name = 'Read' if index == 1 else 'Edit'
                    data = {'file_path': str(target)}
                    if name == 'Edit':
                        data.update(old_string='before confirmation', new_string='after confirmation')
                    send('content_block_start', {'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'tool_use', 'id': 'tool_wk_' + str(index), 'name': name, 'input': {}}})
                    send('content_block_delta', {'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'input_json_delta', 'partial_json': json.dumps(data)}})
                    stop = 'tool_use'
                else:
                    send('content_block_start', {'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'text', 'text': ''}})
                    send('content_block_delta', {'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'text_delta', 'text': 'Synthetic host fixture completed.'}})
                    stop = 'end_turn'
                send('content_block_stop', {'type': 'content_block_stop', 'index': 0})
                send('message_delta', {'type': 'message_delta', 'delta': {'stop_reason': stop, 'stop_sequence': None}, 'usage': {'output_tokens': 10}})
                send('message_stop', {'type': 'message_stop'})
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        hook = state / 'hook.py'
        marker = state / 'hook-invoked'
        if case == 'missing_enrollment':
            hook.write_text('import pathlib,subprocess,sys\npathlib.Path(' + repr(str(marker)) + ').write_text("invoked")\nr=subprocess.run(' + repr([str(AGENT), '--state-dir', str(state), 'protect-hook', '--client', 'claude-code', '--version', VERSION]) + ',input=sys.stdin.buffer.read(),capture_output=True)\nsys.stdout.buffer.write(r.stdout)\nsys.exit(r.returncode)\n')
        elif case == 'hook_crash':
            hook.write_text('import pathlib,sys\npathlib.Path(' + repr(str(marker)) + ').write_text("invoked")\nsys.exit(1)\n')
        elif case == 'hook_timeout':
            hook.write_text('import pathlib,time\npathlib.Path(' + repr(str(marker)) + ').write_text("invoked")\ntime.sleep(10)\n')
        settings = state / 'settings.json'
        config = {} if case == 'baseline' else {'hooks': {'PreToolUse': [{'matcher': 'Edit', 'hooks': [{'type': 'command', 'command': shlex.quote(sys.executable) + ' ' + shlex.quote(str(hook)), 'onFailure': 'block', 'timeout': 2}]}]}}
        settings.write_text(json.dumps(config))
        mcp = state / 'mcp.json'
        mcp.write_text('{"mcpServers":{}}')
        env = dict(os.environ)
        for key in list(env):
            if key.startswith(('ANTHROPIC_', 'CLAUDE_')):
                env.pop(key)
        env.update(ANTHROPIC_API_KEY='test', ANTHROPIC_BASE_URL='http://127.0.0.1:' + str(server.server_port), CLAUDE_CONFIG_DIR=str(state / 'claude'), CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1', DISABLE_TELEMETRY='1', DISABLE_ERROR_REPORTING='1', DISABLE_AUTOUPDATER='1')
        try:
            result = subprocess.run([str(CLAUDE), '-p', 'Use Read then Edit to replace before confirmation with after confirmation in ' + str(target) + '.', '--restricted', '--setting-sources', '', '--settings', str(settings), '--strict-mcp-config', '--mcp-config', str(mcp), '--no-session-persistence', '--tools', 'Read,Edit', '--allowedTools', 'Read,Edit', '--permission-mode', 'acceptEdits', '--output-format', 'json', '--max-turns', '4'], cwd=repo, env=env, capture_output=True, text=True, timeout=60)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
        final_bytes = target.read_bytes()
        changed = final_bytes != original_bytes
        expected_bytes = b'after confirmation\n' if case == 'baseline' else original_bytes
        record = {'case': case, 'claude_version': VERSION, 'exit_code': result.returncode, 'model_requests': count[0], 'hook_invoked': marker.exists(), 'target_changed': changed, 'expected_unchanged': case != 'baseline'}
        results.append(record)
        print(json.dumps(record), flush=True)
        if result.returncode or count[0] < 3 or final_bytes != expected_bytes or (case != 'baseline' and (not marker.exists())):
            print('Fixture failed; client output is intentionally withheld.', flush=True)
            raise SystemExit(1)
print(json.dumps({'passed': True, 'scope': 'actual Claude host with synthetic loopback model only; no cloud, real credentials or browser acknowledgement', 'cases': results}))
