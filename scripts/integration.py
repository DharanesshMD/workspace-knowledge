"""Real PostgreSQL + HTTP + stdio MCP smoke test. Requires WK_TEST_DATABASE_URL."""
import asyncio
import json
import os
from pathlib import Path
import secrets
import urllib.request
import urllib.error

ROOT = Path(__file__).resolve().parents[1]

def post(url, token, body):
    request = urllib.request.Request(url, data=json.dumps(body).encode(), headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=5) as response:
        return json.load(response)

async def main():
    database = os.environ['WK_TEST_DATABASE_URL']
    assert database.split('?')[0].endswith('/wk_test'), 'requires disposable wk_test database'
    suffix = '.exe' if os.name == 'nt' else ''
    token = secrets.token_urlsafe(32)
    env = dict(os.environ, WK_DATABASE_URL=database, WK_DEV_TOKEN=token, WK_PORT='0')
    server = await asyncio.create_subprocess_exec(str(ROOT / 'target/debug' / ('wk-server' + suffix)), '--dev', env=env, stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.PIPE)
    mcp = None
    try:
        line = (await asyncio.wait_for(server.stderr.readline(), 15)).decode()
        assert 'http://127.0.0.1:' in line, 'server did not start: ' + line
        base = line.split('http://', 1)[1].split(' ', 1)[0]
        base = 'http://' + base
        try:
            post(base + '/v1/search', 'invalid', {'product': 'demo', 'query': 'limiter', 'limit': 5})
            raise AssertionError('unauthenticated request accepted')
        except urllib.error.HTTPError as exc:
            assert exc.code == 401
        marker = 'limiter' + secrets.token_hex(8)
        body = {'schema_version': 1, 'idempotency_key': marker, 'policy_revision': 1, 'title': marker, 'intent': 'Synthetic tenant rate limiter', 'rationale': 'Reuse shared middleware', 'decisions': ['Tenant-aware limits'], 'constraints': [], 'open_questions': [], 'service_ids': ['backend'], 'evidence': [], 'lifecycle': 'proposed', 'capture_completeness': 'complete', 'base_revision': None}
        card = post(base + '/v1/cards', token, {'product': 'demo', 'card': body})['card']
        query = {'product': 'demo', 'query': marker, 'limit': 5}
        assert post(base + '/v1/search', token, query)['cards'] == []
        post(base + '/v1/cards/' + card['id'] + '/publish', token, {'product': 'demo'})
        assert len(post(base + '/v1/search', token, query)['cards']) == 1
        env.update(WK_SERVER_URL=base, WK_PRODUCT='demo')
        mcp = await asyncio.create_subprocess_exec(str(ROOT / 'target/debug' / ('wk-mcp' + suffix)), env=env, stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
        async def rpc(message):
            mcp.stdin.write((json.dumps(message) + '\n').encode())
            await mcp.stdin.drain()
            if 'id' not in message:
                return None
            for _ in range(10):
                line = await asyncio.wait_for(mcp.stdout.readline(), 5)
                assert line, 'MCP closed before responding'
                value = json.loads(line)
                if value.get('id') == message['id']:
                    assert 'error' not in value, value
                    return value['result']
            raise AssertionError('missing RPC response')
        await rpc({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': '2025-03-26', 'capabilities': {}, 'clientInfo': {'name': 'wk-synthetic-test', 'version': '0.1'}}})
        await rpc({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
        listing = await rpc({'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'})
        assert {'knowledge.search', 'knowledge.get', 'knowledge.context'} <= {t['name'] for t in listing['tools']}
        result = await rpc({'jsonrpc': '2.0', 'id': 3, 'method': 'tools/call', 'params': {'name': 'knowledge.search', 'arguments': {'query': marker, 'limit': 5}}})
        assert not result.get('isError', False)
        assert card['id'] in json.dumps(result)
        context = await rpc({'jsonrpc': '2.0', 'id': 4, 'method': 'tools/call', 'params': {'name': 'knowledge.context', 'arguments': {'query': marker, 'budget_tokens': 200}}})
        assert card['id'] in json.dumps(context)
        print('PASS: real PostgreSQL, HTTP authentication, draft/publication, MCP initialization, discovery, search and bounded context')
    finally:
        for process in (mcp, server):
            if process and process.returncode is None:
                process.terminate()
                try:
                    await asyncio.wait_for(process.wait(), 5)
                except asyncio.TimeoutError:
                    process.kill()
                    await process.wait()

if __name__ == '__main__':
    asyncio.run(main())
