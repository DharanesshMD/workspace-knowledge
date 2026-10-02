"""Replay one project's real chat history as two developers and score overlap notices.

Input: the JSONL written by `wk cursor-history cards` (chronological, one line per chat).
The first chats act as Developer A (published cards; the newest A chat is still "in progress"),
the remaining chats act as Developer B starting new work. For each B chat we query the
knowledge service the way B's agent would and score the notices against ground truth:
A chats that changed at least one of the same files.

Uses a disposable `wk_test` database and a local dev server only. Writes a private report.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def call(base, token, method, path, body=None):
    request = urllib.request.Request(base + path, method=method, data=None if body is None else json.dumps(body).encode(),
                                     headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read() or b'{}')


def commits(repo, files, observed_ms):
    """The commit that best contains this chat's changes: made between 1 h before and 48 h after
    the chat ended, containing at least 60% of its distinctive files (containment, not Jaccard,
    because one commit often bundles several sessions). Read-only git log; none if unsure."""
    files = distinctive(files)
    if not files or not observed_ms:
        return []
    end = observed_ms / 1000
    since = datetime.datetime.fromtimestamp(end - 3600, datetime.timezone.utc).isoformat()
    until = datetime.datetime.fromtimestamp(end + 48 * 3600, datetime.timezone.utc).isoformat()
    out = subprocess.run(['git', '-C', repo, 'log', '--format=@%H %ct', '--name-only', f'--since={since}', f'--until={until}'],
                         capture_output=True, text=True)
    if out.returncode != 0:
        return []
    candidates, current = [], None
    for line in out.stdout.splitlines():
        if line.startswith('@'):
            sha, stamp = line[1:].split()
            current = (sha, int(stamp), set())
            candidates.append(current)
        elif line.strip() and current:
            current[2].add(line.strip())
    scored = [(len(files & touched) / len(files), -abs(stamp - end), sha) for sha, stamp, touched in candidates]
    best = max((c for c in scored if c[0] >= 0.6), default=None)
    return [best[2]] if best else []


LOCKFILES = {'todo.md', 'lessons.md', 'package-lock.json', 'pnpm-lock.yaml', 'yarn.lock', 'Cargo.lock', 'bun.lockb', 'poetry.lock'}


def distinctive(files):
    """Mirror of the server's bookkeeping rule: shared plan/task/lock files are not overlaps."""
    return {f for f in files if not (f.startswith(('tasks/', '.superpowers/', 'docs/superpowers/')) or f.rsplit('/', 1)[-1] in LOCKFILES)}


def score(returned, truth):
    hit = returned & truth
    return {'returned': len(returned), 'relevant': len(truth), 'hits': len(hit),
            'recall': round(len(hit) / len(truth), 2) if truth else None,
            'precision': round(len(hit) / len(returned), 2) if returned else None}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('cards', type=Path)
    parser.add_argument('--repo', required=True)
    parser.add_argument('--database', default=os.environ.get('WK_TEST_DATABASE_URL'))
    parser.add_argument('--developer-a', type=int, default=7, help='number of earliest chats acting as Developer A')
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    if not args.database or not args.database.split('?')[0].endswith('/wk_test'):
        sys.exit('requires a disposable wk_test database (WK_TEST_DATABASE_URL)')
    chats = [json.loads(line) for line in args.cards.read_text().splitlines() if line.strip()]
    a_chats, b_chats = chats[:args.developer_a], chats[args.developer_a:]

    token_a, token_b = secrets.token_urlsafe(32), secrets.token_urlsafe(32)
    env = dict(os.environ, WK_DATABASE_URL=args.database, WK_DEV_TOKEN=token_a, WK_DEV_TOKEN_B=token_b, WK_PORT='0')
    suffix = '.exe' if os.name == 'nt' else ''
    server = subprocess.Popen([str(ROOT / 'target/debug' / ('wk-server' + suffix)), '--dev'], env=env,
                              stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    report = {'project_repo': args.repo, 'developer_a_chats': len(a_chats), 'developer_b_chats': len(b_chats),
              'loading': [], 'sessions': []}
    try:
        line = server.stderr.readline()
        if 'http://127.0.0.1:' not in line:
            sys.exit('server did not start: ' + line)
        base = 'http://' + line.split('http://', 1)[1].split(' ', 1)[0]

        # Load Developer A's published work, linked to commits.
        card_of = {}
        for chat in a_chats:
            entry = {'id': chat['id'], 'files_changed': len(chat['files_changed'])}
            card = chat.get('card')
            if not card:
                entry['skipped'] = chat.get('error', 'no card')
                report['loading'].append(entry)
                continue
            shas = commits(args.repo, chat['files_changed'], chat.get('observed_at_ms'))
            card['evidence'] = card['evidence'] + [{'kind': 'commit', 'reference': f'commit:{sha}'} for sha in shas]
            status, body = call(base, token_a, 'POST', '/v1/cards', {'product': 'demo', 'card': card})
            if status != 200:
                entry['rejected'] = body.get('error', status)
                report['loading'].append(entry)
                continue
            card_id = body['card']['id']
            status, _ = call(base, token_a, 'POST', f'/v1/cards/{card_id}/publish', {'product': 'demo'})
            entry.update(card_id=card_id, published=status == 200, commits=len(shas), title=card['title'], coverage=card['capture_completeness'])
            if status == 200:  # only published work can be notified about
                card_of[chat['id']] = card_id
            report['loading'].append(entry)

        # The newest A chat is treated as still in progress.
        current = next((c for c in reversed(a_chats) if c.get('card') and c['files_changed']), None)
        if current:
            intent = current['card']['title'][:280]
            status, body = call(base, token_a, 'POST', '/v1/products/demo/activity',
                                {'session_id': current['id'][:128], 'services': ['frontend'],
                                 'files': current['files_changed'][:200], 'intent': intent})
            report['in_progress'] = {'id': current['id'], 'intent': intent, 'recorded': status == 200, 'error': body.get('error')}

        # Developer B starts each newer chat: ask before acting.
        for chat in b_chats:
            changed = set(chat['files_changed'])
            truth = {card_of[a['id']] for a in a_chats if a['id'] in card_of and distinctive(changed) & distinctive(a['files_changed'])}
            active_truth = bool(current and distinctive(changed) & distinctive(current['files_changed']))
            session = {'id': chat['id'], 'files_changed': len(changed), 'relevant_a_cards': len(truth),
                       'a_in_progress_overlaps': active_truth}
            request = chat.get('first_request') or ''
            moments = {'prompt_time': {'query': request}, 'edit_time': {'files': sorted(changed)[:100]}}
            for moment, query in moments.items():
                if not any(query.values()):
                    session[moment] = {'skipped': 'nothing known at this moment'}
                    continue
                started = time.perf_counter()
                status, body = call(base, token_b, 'POST', '/v1/products/demo/overlaps', query)
                elapsed = round((time.perf_counter() - started) * 1000)
                notices = body.get('notices', [])
                returned = {n['card_id'] for n in notices if n.get('card_id')}
                session[moment] = {'status': status, 'ms': elapsed, 'query_ignored': body.get('query_ignored'),
                                   'kinds': [f"{n['kind']}/{n['matched_by']}" for n in notices],
                                   'active_notice': any(n['kind'] == 'active' for n in notices),
                                   'score': score(returned, truth), 'text': body.get('text', '')}
            # Baseline: existing all-words search on the request.
            if request:
                status, body = call(base, token_b, 'POST', '/v1/search', {'product': 'demo', 'query': request[:1024], 'limit': 8})
                found = {c['id'] for c in body.get('cards', [])} if status == 200 else set()
                session['baseline_search'] = {'status': status, 'error': body.get('error'), 'score': score(found, truth)}
            report['sessions'].append(session)
    finally:
        server.terminate()
        server.wait(timeout=10)
    fd = os.open(args.report, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as out:
        json.dump(report, out, indent=2)
    print(f'Report written to {args.report}')


if __name__ == '__main__':
    main()
