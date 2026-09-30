"""Exercise the actual CLI with temporary, synthetic data only."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

binary = Path(__file__).resolve().parents[1] / 'target' / 'debug' / ('wk.exe' if os.name == 'nt' else 'wk')
if not binary.exists():
    raise SystemExit('Build first: cargo build -p wk-cli')
with tempfile.TemporaryDirectory(prefix='wk-demo-') as directory:
    base = Path(directory)
    product = base / 'product'
    backend = product / 'backend'
    backend.mkdir(parents=True)
    state = base / 'private-state'

    def run(*args):
        result = subprocess.run([str(binary), '--state-dir', str(state), *map(str, args)], check=True, text=True, capture_output=True)
        value = json.loads(result.stdout)
        print(json.dumps(value, indent=2))
        return value

    run('workspace', 'approve', product, '--product', 'synthetic-demo', '--service', 'backend', '--yes')
    assert run('workspace', 'check', product, backend)['eligible']
    assert not run('workspace', 'check', product, base)['eligible']
    events = []
    for sequence, kind in enumerate(['user_prompt', 'assistant_response', 'turn_end'], start=1):
        events.append({'schema_version': 1, 'event_id': str(sequence), 'session_id': 'demo-session', 'agent_id': 'demo-agent', 'turn_id': 'turn-1', 'sequence': sequence, 'kind': kind, 'path': str(backend), 'text': 'Synthetic conversation; no real user data.'})
    fixture = base / 'synthetic.jsonl'
    fixture.write_text('\n'.join(json.dumps(event) for event in events), encoding='utf-8')
    report = run('replay', product, fixture)
    assert report['turns'][0]['complete']
    assert not report['published']
    run('workspace', 'pause', product)
    assert not run('workspace', 'check', product, backend)['eligible']
    print('Synthetic consent, scope, replay, and pause checks passed.')
