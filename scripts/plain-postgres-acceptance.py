"""Disposable ordinary PostgreSQL acceptance; requires Docker and built binaries."""
import json
import os
from pathlib import Path
import secrets
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]

def main():
    name = 'wk-plain-' + secrets.token_hex(6)
    password = secrets.token_hex(24)
    def docker(*args):
        return subprocess.check_output(['docker', *args], text=True).strip()
    try:
        docker('run', '-d', '--name', name, '--label', 'wk.fixture=plain-postgres',
               '-e', 'POSTGRES_USER=wk_test', '-e', 'POSTGRES_DB=wk_test',
               '-e', 'POSTGRES_PASSWORD=' + password, '-p', '127.0.0.1::5432', 'postgres:17')
        # TCP readiness: the image's init-time server listens only on the Unix socket
        # and restarts afterwards, so a socket check can pass before the real server.
        for _ in range(120):
            if subprocess.run(['docker', 'exec', name, 'pg_isready', '-h', '127.0.0.1', '-U', 'wk_test'],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
                break
            time.sleep(.5)
        else:
            raise RuntimeError('ordinary PostgreSQL startup deadline')
        port = docker('port', name, '5432/tcp').rsplit(':', 1)[1]
        env = {k: v for k, v in os.environ.items() if not k.startswith('WK_')}
        env['WK_TEST_DATABASE_URL'] = f'postgres://wk_test:{password}@127.0.0.1:{port}/wk_test'
        # Existing HTTP + both MCP transports, publication and supersession.
        subprocess.run(['python3', 'scripts/integration.py'], cwd=ROOT, env=env, check=True)
        # Complete configuration with no optional extension must still work.
        env.update(WK_EMBEDDING_URL='http://127.0.0.1:1/api/embed',
                   WK_EMBEDDING_MODEL='nomic-embed-text:v1.5',
                   WK_EMBEDDING_DIGEST='a' * 64, WK_EMBEDDING_DIMENSIONS='768',
                   WK_EXPECT_RETRIEVAL_REASON='index_not_ready')
        subprocess.run(['python3', 'scripts/integration.py'], cwd=ROOT, env=env, check=True)
        sql = "SELECT count(*) FROM pg_extension WHERE extname='vector';"
        assert docker('exec', name, 'psql', '-U', 'wk_test', '-Atc', sql) == '0'
        print('PASS: ordinary PostgreSQL, disabled search and configured missing-index fallback')
    finally:
        inspected = subprocess.run(['docker', 'inspect', name], capture_output=True, text=True)
        if inspected.returncode == 0:
            info = json.loads(inspected.stdout)[0]
            if info['Config']['Labels'].get('wk.fixture') == 'plain-postgres':
                subprocess.run(['docker', 'rm', '-f', '-v', name], check=True, stdout=subprocess.DEVNULL)

if __name__ == '__main__':
    main()
