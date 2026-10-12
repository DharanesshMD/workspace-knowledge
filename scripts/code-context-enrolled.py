"""Real local portal/native collector/Claude-host acceptance using synthetic data only.

Requires an explicit supported WK_TEST_CLAUDE, optional WK_TEST_AGENT executable,
and owned WK_TEST_DATABASE_URL ending in /wk_test. Never reads global profiles.
"""
import hashlib
import importlib.util
import json
import os
import plistlib
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
import uuid
from claude_fixture import ModelFixture

spec = importlib.util.spec_from_file_location('native_fixture', Path(__file__).with_name('portal-device-smoke.py'))
native_fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_fixture)
identity = native_fixture.identity
ROOT = identity.REPO
CLAUDE = Path(os.environ['WK_TEST_CLAUDE']).resolve(strict=True)
AGENT = Path(os.environ.get('WK_TEST_AGENT', ROOT / 'target/debug/wk-agent')).resolve(strict=True)
VERSION = subprocess.check_output([str(CLAUDE), '--version'], text=True, timeout=10).split()[0]
if tuple(map(int, VERSION.split('.'))) < (2, 1, 295):
    raise SystemExit('Explicit Claude 2.1.295+ required before any enrollment')


def acceptance(origin, owner, mutation, command, provider, start):
    assert os.uname().sysname == 'Darwin', 'Owned native Keychain cleanup currently targets macOS'
    org = json.loads(owner.request(origin + '/v1/me')[2])['org']
    root_api = origin + '/v1/products/alpha'
    command('provision', {'issuer': provider.cfg['issuer'], 'subject': 'bob', 'org': org, 'user': 'bob', 'product': 'alpha', 'services': ['backend'], 'can_review': True})
    command('admin-assign', {'org': org, 'user': 'bob', 'product': 'alpha', 'services': ['backend']})
    provider.cfg['subject'] = 'bob'
    bob = identity.Browser()
    assert bob.request(start(bob))[0] == 303
    provider.cfg['subject'] = 'alice'
    bob_me = json.loads(bob.request(origin + '/v1/me')[2])
    bob_headers = {'Origin': origin, 'Content-Type': 'application/json', 'X-CSRF-Token': bob_me['csrf_token']}

    def post(browser, path, value, headers):
        status, _, raw = browser.request(path, value, headers)
        assert status == 200, 'Synthetic provenance operation returned ' + str(status)
        return json.loads(raw)

    with tempfile.TemporaryDirectory(prefix='wk-enrolled-context-') as directory:
        work = Path(directory).resolve()
        work.chmod(0o700)
        repo = work / 'repo'
        (repo / 'backend').mkdir(parents=True)
        target = repo / 'backend/retry.rs'
        original = b'before confirmation\n'
        target.write_bytes(original)
        def git(*args):
            return subprocess.check_output(['git', '-C', str(repo), '-c', 'user.name=Synthetic Fixture', '-c', 'user.email=fixture@example.invalid', *args], text=True).strip()
        git('init', '--initial-branch=main'); git('add', '.'); git('commit', '-m', 'synthetic invariant')
        revision = git('rev-parse', 'HEAD')
        approval_dir, state = work / 'approval', work / 'native'
        collector_port = identity.allocate()
        subprocess.run([str(ROOT/'target/debug/wk'), '--state-dir', str(approval_dir), 'workspace', 'approve', str(repo), '--product', 'alpha', '--service', 'backend', '--yes'], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        binding = approval_dir / (hashlib.sha256(str(repo).encode()).hexdigest() + '.json')
        env = {k: v for k, v in os.environ.items() if not k.startswith(('ANTHROPIC_', 'CLAUDE_'))}
        env.update(PATH=str(Path(os.environ['WK_TEST_CLAUDE']).absolute().parent) + os.pathsep + os.environ['PATH'], CLAUDE_CONFIG_DIR=str(work/'claude'), CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1', DISABLE_TELEMETRY='1', DISABLE_ERROR_REPORTING='1', DISABLE_AUTOUPDATER='1')
        proc = None
        def authorize(url):
            browser = identity.Browser()
            status, headers, _ = browser.request(url)
            assert status == 302
            assert browser.request(headers['Location'])[0] == 200
        def cli(*args, payload=None):
            result = subprocess.run([str(AGENT), '--state-dir', str(state), *args], env=env, input=None if payload is None else json.dumps(payload), capture_output=True, text=True, timeout=120)
            if result.returncode:
                receipt = json.loads((state/'device-receipt.json').read_text())
                matches = receipt['config_hash'] == hashlib.sha256((state/'agent.json').read_bytes()).hexdigest()
                raise AssertionError('Owned collector phase '+args[0]+' failed; configuration matches approval receipt: '+str(matches)+'; raw output withheld')
            return json.loads(result.stdout)
        try:
            proc = subprocess.Popen([str(AGENT), '--state-dir', str(state), 'device', 'connect', '--binding', str(binding), '--service', 'backend', '--server', origin, '--port', str(collector_port), '--yes', '--no-browser'], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            authorize(json.loads(proc.stderr.readline())['authorization_url'])
            print('Owned collector sign-in returned; waiting for synthetic Keychain storage if prompted.', flush=True)
            stdout, _ = proc.communicate(timeout=300)
            assert proc.returncode == 0, 'Owned collector enrollment failed; raw output withheld'
            device_id = json.loads(stdout)['device_id']
            # Exercise the shipped launchd contract using only a unique temporary job.
            label = 'org.workspace-knowledge.acceptance.'+uuid.uuid4().hex
            domain = 'gui/'+str(os.getuid())
            job = plistlib.loads((ROOT/'deploy/launchd/org.workspace-knowledge.agent.plist').read_bytes())
            job.update(Label=label, ProgramArguments=[str(AGENT), '--state-dir', str(state), 'serve'], StandardOutPath=str(work/'launch.stdout'), StandardErrorPath=str(work/'launch.stderr'))
            plist = work/'owned-agent.plist'; plist.write_bytes(plistlib.dumps(job)); plist.chmod(0o600)
            try:
                for _ in range(2):
                    subprocess.run(['launchctl', 'bootstrap', domain, str(plist)], check=True, capture_output=True)
                    for attempt in range(30):
                        health = cli('doctor')['collector']
                        if health.get('available'): break
                        time.sleep(.2)
                    else: raise AssertionError('Owned native startup did not become healthy')
                    subprocess.run(['launchctl', 'bootout', domain+'/'+label], check=True, capture_output=True)
                    assert subprocess.run(['launchctl', 'print', domain+'/'+label], capture_output=True).returncode != 0
                print(json.dumps({'owned_launchd_start_stop_restart': True, 'apple_signing_verified': False}), flush=True)
            finally:
                subprocess.run(['launchctl', 'bootout', domain+'/'+label], capture_output=True)
            repository_id = str(uuid.uuid4())
            def admin(kind, name, value):
                now = int(time.time())
                claims = {'iss': provider.cfg['issuer'], 'aud': 'wk-collector', 'sub': 'bob', 'iat': now, 'exp': now+300}
                signed = identity.b64(json.dumps({'alg': 'RS256', 'kid': 'synthetic-portal'}).encode())+'.'+identity.b64(json.dumps(claims).encode())
                signature = subprocess.check_output([native_fixture.OPENSSL, 'dgst', '-sha256', '-sign', provider.cfg['pem']], input=signed.encode(), stderr=subprocess.DEVNULL)
                admin_token = signed+'.'+identity.b64(signature)
                admin_env = dict(env, WK_DATABASE_URL=os.environ['WK_TEST_DATABASE_URL'], WK_ADMIN_TEST_ISSUER=provider.cfg['issuer'], WK_ADMIN_TEST_AUDIENCE='wk-collector', WK_ADMIN_TEST_JWKS_URL=provider.cfg['issuer']+'/keys')
                result = subprocess.run([str(ROOT/'target/debug/wk-server'), '--dev-admin'], env=admin_env, input=json.dumps({'token': admin_token, 'product': 'alpha', 'command': {'kind': kind, name: value}}), capture_output=True, text=True, timeout=30)
                assert result.returncode == 0, 'Synthetic repository administration failed; raw output withheld'
                return json.loads(result.stdout)
            admin('code-repository', 'repository', {'id': repository_id, 'services': ['backend'], 'high_risk_paths': ['backend/retry.rs'], 'contact_hosts': ['team.example.invalid'], 'base_revision': None})
            mapping = admin('code-mapping', 'mapping', {'repository_id': repository_id, 'user_id': 'bob', 'identity_reference': 'team:bob', 'proof_reference': 'Synthetic team attestation', 'services': ['backend'], 'base_revision': None})['mapping']
            post(bob, root_api+'/code-profile', {'display_name': 'Bob Synthetic', 'contact_url': 'https://team.example.invalid/bob', 'services': ['backend'], 'base_revision': None}, bob_headers)
            targets = [{'path': 'backend/retry.rs', 'start': None, 'end': None}]
            contribution = post(bob, root_api+'/code-contributions', {'repository_id': repository_id, 'mapping_id': mapping['id'], 'code_revision': revision, 'targets': targets}, bob_headers)
            contribution = post(bob, root_api+'/code-contributions/'+contribution['id']+'/confirm', {'expected_revision': contribution['revision'], 'confirmed': True}, bob_headers)
            contribution = post(bob, root_api+'/code-contributions/'+contribution['id']+'/review', {'expected_revision': contribution['revision']}, bob_headers)
            body = {'schema_version': 1, 'idempotency_key': secrets.token_hex(16), 'policy_revision': 1, 'title': 'Synthetic retry invariant', 'intent': 'Prevent duplicate billing', 'rationale': 'Keep the original operation id because the billing worker deduplicates by that id', 'decisions': [], 'constraints': ['Preserve the operation id across retries'], 'open_questions': [], 'service_ids': ['backend'], 'evidence': [], 'lifecycle': 'proposed', 'capture_completeness': 'complete', 'base_revision': None}
            card = post(bob, origin+'/v1/cards', {'product': 'alpha', 'card': body}, bob_headers)['card']
            post(bob, origin+'/v1/cards/'+card['id']+'/publish', {'product': 'alpha'}, bob_headers)
            anchor = post(bob, root_api+'/code-anchors', {'repository_id': repository_id, 'card_id': card['id'], 'card_revision': card['revision'], 'code_revision': revision, 'targets': targets, 'contribution_id': contribution['id']}, bob_headers)
            anchor = post(bob, root_api+'/code-anchors/'+anchor['id']+'/confirm', {'expected_revision': anchor['revision'], 'confirmed': True}, bob_headers)
            post(bob, root_api+'/code-anchors/'+anchor['id']+'/review', {'expected_revision': anchor['revision']}, bob_headers)
            cli('repository-enroll', '--repository-id', repository_id, '--yes')
            settings = work / 'settings.json'
            settings.write_text('{}')
            cli('protect', 'enroll', '--settings-file', str(settings), '--yes')
            cases = []
            def host(tool, new='after confirmation'):
                model = ModelFixture(target, tool, new=new).start()
                client_env = {k: v for k, v in env.items() if not k.startswith(('ANTHROPIC_', 'CLAUDE_'))}
                client_env.update(ANTHROPIC_API_KEY='test', ANTHROPIC_BASE_URL='http://127.0.0.1:'+str(model.server_port), CLAUDE_CONFIG_DIR=str(work/'claude'), CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1', DISABLE_TELEMETRY='1', DISABLE_ERROR_REPORTING='1', DISABLE_AUTOUPDATER='1')
                mcp = work/'mcp.json'; mcp.write_text('{"mcpServers":{}}')
                try:
                    result = subprocess.run([str(CLAUDE), '-p', 'Use Read then '+tool+' on '+str(target), '--restricted', '--setting-sources', '', '--settings', str(settings), '--strict-mcp-config', '--mcp-config', str(mcp), '--no-session-persistence', '--tools', 'Read,'+tool, '--allowedTools', 'Read,'+tool, '--permission-mode', 'acceptEdits', '--output-format', 'json', '--max-turns', '4'], cwd=repo, env=client_env, capture_output=True, text=True, timeout=60)
                finally: model.stop()
                assert result.returncode == 0 and model.requests >= 3 and tool in model.offered_tools, 'Actual host fixture failed; raw output withheld'
            def pending(): return json.loads((state/'pending-code-operation.json').read_text())
            def acknowledge():
                value = pending()
                subprocess.run(['node', str(ROOT/'web/scripts/code-context-live.mjs'), origin, value['brief_id']], check=True, timeout=60)
                return value
            for tool in ['Edit', 'Write']:
                target.write_bytes(original)
                host(tool)
                assert target.read_bytes() == original
                value = acknowledge()
                host(tool)
                assert target.read_bytes() == b'after confirmation\n'
                assert not (state/'pending-code-operation.json').exists()
                view = json.loads(owner.request(root_api+'/preflights/'+value['brief_id'])[2])
                assert view['consumed'] and view['acknowledged']
                cases.append({'tool': tool, 'initial_denied': True, 'browser_acknowledged': True, 'matching_retry_changed_target': True, 'receipt_consumed_once': True})
            target.write_bytes(original); host('Edit'); acknowledge()
            host('Edit', 'changed operation')
            assert target.read_bytes() == original
            cases.append({'case': 'changed_input', 'blocked': True})
            acknowledge(); target.write_bytes(original+b'// source changed\n'); host('Edit', 'changed operation')
            assert target.read_bytes() == original+b'// source changed\n'
            cases.append({'case': 'changed_source', 'blocked': True})
            target.write_bytes(original); host('Edit'); old = acknowledge()
            expired = pending(); expired['expires_at_ms'] = 0
            (state/'pending-code-operation.json').write_text(json.dumps(expired))
            host('Edit')
            assert target.read_bytes() == original and pending()['brief_id'] != old['brief_id']
            cases.append({'case': 'expired_local_receipt', 'blocked': True})
            acknowledge()
            # Revoke the real owned device through the browser-authorized owner API.
            devices = json.loads(owner.request(root_api+'/devices')[2])['devices']
            device = next(item for item in devices if item['id'] == device_id)
            post(owner, root_api+'/devices/'+device_id+'/revoke', {'expected_revision': device['revision']}, mutation)
            before = target.read_bytes(); host('Edit')
            assert target.read_bytes() == before
            cases.append({'case': 'revoked_device', 'blocked': True})
            print(json.dumps({'enrolled_context_cases': cases, 'claude_version': VERSION, 'production_verified': False}), flush=True)
            return {'real_enrolled_host_browser_retry': True, 'changed_input_source_expiry_and_revocation_blocked': True, 'owned_launchd_start_stop_restart': True}
        finally:
            if proc is not None and proc.poll() is None: proc.terminate(); proc.wait(timeout=10)
            account = hashlib.sha256(str(state.resolve()).encode()).hexdigest()
            for purpose in ['capture-key', 'ipc-key', 'device-key', 'server-token']:
                subprocess.run(['/usr/bin/security', 'delete-generic-password', '-s', 'workspace-knowledge', '-a', account+':'+purpose], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == '__main__':
    identity.main(acceptance, native_fixture.NativeProvider, collector=True)
