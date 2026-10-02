"""Synthetic TLS OpenBao integration; uses only a disposable wk-final-bao container."""
import json
import os
from pathlib import Path
import ssl
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]

def main():
    port = subprocess.check_output(['docker', 'port', 'wk-final-bao', '8200/tcp'], text=True).strip().rsplit(':', 1)[1]
    base = f'https://127.0.0.1:{port}'
    with tempfile.TemporaryDirectory(prefix='wk-bao-') as directory:
        root = Path(directory).resolve()
        ca = root / 'ca.pem'
        for attempt in range(30):
            copied = subprocess.run(['docker', 'cp', 'wk-final-bao:/tmp/wk-tls/vault-ca.pem', str(ca)], capture_output=True)
            if copied.returncode == 0:
                break
            time.sleep(1)
        else:
            raise RuntimeError('Disposable TLS vault did not create its CA')
        context = ssl.create_default_context(cafile=str(ca))
        def api(path, body):
            request = urllib.request.Request(base + '/v1/' + path, data=json.dumps(body).encode(), headers={'X-Vault-Token': 'wk-synthetic-root', 'Content-Type': 'application/json'})
            with urllib.request.urlopen(request, context=context, timeout=10) as response:
                data = response.read()
                return json.loads(data) if data else {}
        for attempt in range(30):
            try:
                with urllib.request.urlopen(base + '/v1/sys/health', context=context, timeout=2):
                    break
            except (OSError, urllib.error.URLError):
                time.sleep(1)
        else:
            raise RuntimeError('Disposable TLS vault did not become healthy')
        api('sys/policies/acl/wk-test', {'policy': 'path "secret/data/team/backend/development" { capabilities = ["read", "create", "update"] }'})
        token = api('auth/token/create', {'policies': ['wk-test'], 'no_default_policy': True})['auth']['client_token']
        profile = root / 'profile.json'
        profile.write_text(json.dumps({'service': 'backend', 'profile': 'development', 'mount': 'secret', 'path': 'team/backend/development', 'address': base, 'workspace': str(root), 'keys': ['DEMO_SECRET']}))
        source = root / '.env'
        source.write_text('DEMO_SECRET=synthetic-secret-one\n')
        env = dict(os.environ, WK_BAO_TOKEN=token, WK_BAO_CA_FILE=str(ca))
        command = [str(ROOT / 'target/debug/wk-env'), '--config', str(profile)]
        def run(args):
            return subprocess.run(command + args, env=env, capture_output=True, text=True)
        assert run(['import', '--file', str(source), '--approve-file-import']).returncode == 0
        assert run(['import', '--file', str(source), '--approve-file-import']).returncode != 0, 'CAS overwrite accepted'
        def child(expected):
            import sys
            result = run(['run', sys.executable, '-c', f'import os; assert os.environ["DEMO_SECRET"] == {expected!r}; assert "WK_BAO_TOKEN" not in os.environ; print(os.environ["DEMO_SECRET"])'])
            assert result.returncode == 0, result.stderr
            assert expected not in result.stdout + result.stderr
            assert '[REDACTED]' in result.stdout
        child('synthetic-secret-one')
        api('secret/data/team/backend/development', {'options': {'cas': 1}, 'data': {'DEMO_SECRET': 'synthetic-secret-two'}})
        child('synthetic-secret-two')
        api('auth/token/revoke', {'token': token})
        assert run(['run', 'echo', 'must-not-run']).returncode != 0
        print('PASS: verified TLS, scoped import, CAS protection, child delivery, output suppression, rotation and revoked-token denial')

if __name__ == '__main__':
    main()
