"""Compare rule, assistant-written and local-model drafts on a synthetic fixture.

Input is the fixture `scripts/fixtures/summary-quality.json` (synthetic turns with the key terms a
reader needs) and the drafts written by the ignored Rust test
`rules_draft::tests::dump_drafts_for_summary_quality_eval`. Two kinds of measure are reported:

* text: does a card's title say what the work was, and do its fields carry the reasons?
* overlap: the cards are loaded into a disposable server through `history-eval.py` and a second
  developer's new work is checked for notices about the first developer's related cards.

Uses only the synthetic fixture. Never point it at real conversations.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import urllib.parse

HERE = Path(__file__).resolve().parent
MODES = ('rules', 'local', 'assistant')
MARK = '[auto-drafted] '
# Titles that tell a reader nothing: greetings, bare confirmations, pasted traces and code fences.
JUNK_TITLE = re.compile(r'^(?:(?:hi|hey|hello|yes|yeah|ok|okay|sure|continue|go ahead|do it|thanks|thank you|quick one|traceback)\b|```|[\w.]*(?:error|exception)\s*[:(])', re.I)
# Courtesies that carry no knowledge but waste a reader's attention when quoted into a card.
PLEASANTRY = re.compile(r"(let me know|hope this helps|happy to help|feel free to|hope you|quick one|go ahead|^done\.?\s)", re.I)


def strip_mark(title):
    return title[len(MARK):] if title.startswith(MARK) else title


def card_text(card, fields=('title', 'intent', 'rationale', 'decisions', 'constraints', 'open_questions')):
    parts = []
    for field in fields:
        value = card.get(field) or ''
        parts.extend(value if isinstance(value, list) else [value])
    return ' '.join(parts).lower()


def mentions(text, stems):
    return [stem for stem in stems if stem.lower() in text]


def score_text(turns, drafts):
    """Per-mode text measures over every turn. A held turn counts as no card, not as a bad one."""
    result = {}
    for mode in MODES:
        cards = [(turn, draft[mode]['card']) for turn, draft in zip(turns, drafts) if 'card' in draft.get(mode, {})]
        n = len(cards)

        def share(values):
            return round(sum(values) / n, 2) if n else None

        result[mode] = {
            'turns': len(turns),
            'cards': n,
            'held': len(turns) - n,
            # The title names the work: it contains at least one of the item's topic stems.
            'title_names_the_work': share([bool(mentions(strip_mark(c['title']).lower(), t['topic'])) for t, c in cards]),
            'junk_title': share([bool(JUNK_TITLE.match(strip_mark(c['title']).strip())) for t, c in cards]),
            'topic_in_title_or_intent': share([len(mentions(card_text(c, ('title', 'intent')), t['topic'])) / len(t['topic']) for t, c in cards]),
            # The reasons: the stems a teammate needs to trust or challenge the work.
            'reasons_anywhere': share([len(mentions(card_text(c), t['why'])) / len(t['why']) for t, c in cards]),
            'reasons_in_rationale': share([len(mentions(card_text(c, ('rationale',)), t['why'])) / len(t['why']) for t, c in cards]),
            'has_decisions_or_constraints': share([bool(c['decisions'] or c['constraints']) for t, c in cards]),
            # What a reader has to read: characters across every field of the card.
            'mean_chars': round(sum(len(card_text(c)) for t, c in cards) / n) if n else None,
            'has_pleasantry': share([bool(PLEASANTRY.search(card_text(c, ('title', 'intent', 'rationale')))) for t, c in cards]),
        }
    return result


def card_lines(turns, drafts, mode, with_files, complete):
    """The cards as `wk cursor-history cards` writes them, one chat per line, chronological.

    `complete` relabels the cards `complete`: a counterfactual, because the server publishes only
    complete cards and rule and assistant drafts are partial (so as shipped none can be published)."""
    lines = []
    for index, (turn, draft) in enumerate(zip(turns, drafts)):
        stamp = 1_790_000_000_000 + index * 3_600_000
        line = {'id': turn['id'], 'observed_at_ms': stamp, 'files_changed': turn['files'], 'files_read': [],
                'first_request': turn['prompt'][:400], 'seconds': 0}
        card = draft.get(mode, {}).get('card')
        if card:
            card = dict(card, observed_at_ms=stamp)
            if complete:
                card['capture_completeness'] = 'complete'
            card['evidence'] = [{'kind': 'file_observation', 'reference': f} for f in turn['files']] if with_files else []
            line['card'] = card
        else:
            line['error'] = draft.get(mode, {}).get('held') or draft.get(mode, {}).get('skipped') or 'no card'
        lines.append(line)
    return lines


def reset_database(url):
    """Recreate the disposable database so every run starts empty (refuses any other name)."""
    parts = urllib.parse.urlsplit(url)
    if parts.path != '/wk_test':
        sys.exit('refusing to reset a database other than wk_test')
    admin = urllib.parse.urlunsplit(parts._replace(path='/postgres', query=''))
    for statement in ('DROP DATABASE IF EXISTS wk_test WITH (FORCE)', 'CREATE DATABASE wk_test'):
        subprocess.run(['psql', admin, '-q', '-v', 'ON_ERROR_STOP=1', '-c', statement], check=True)


def mean(values):
    values = [v for v in values if v is not None]
    return round(sum(values) / len(values), 2) if values else None


def summarize_overlap(report):
    sessions = [s for s in report['sessions'] if s['relevant_a_cards']]
    out = {'related_sessions': len(sessions), 'published': sum(1 for e in report['loading'] if e.get('published')),
           'rejected_or_skipped': sum(1 for e in report['loading'] if not e.get('published'))}
    for moment in ('prompt_time', 'edit_time'):
        scores = [s[moment]['score'] for s in sessions if 'score' in s.get(moment, {})]
        out[moment] = {'recall': mean([x['recall'] for x in scores]), 'precision': mean([x['precision'] for x in scores])}
    baseline = [s['baseline_search']['score'] for s in sessions if 'score' in s.get('baseline_search', {})]
    out['baseline_search'] = {'recall': mean([x['recall'] for x in baseline]), 'precision': mean([x['precision'] for x in baseline])}
    return out


def run_overlap(turns, drafts, work, args):
    """Each mode with and without file evidence, as shipped and as if its cards were publishable."""
    results = {}
    for mode in MODES:
        for with_files in (True, False):
            for complete in (False, True):
                name = f'{mode}-{"files" if with_files else "nofiles"}-{"publishable" if complete else "as-shipped"}'
                lines = card_lines(turns, drafts, mode, with_files, complete)
                if not any('card' in line for line in lines):
                    continue
                cards = work / f'{name}.jsonl'
                cards.write_text(''.join(json.dumps(line) + '\n' for line in lines))
                report = work / f'{name}.report.json'
                reset_database(args.database)
                subprocess.run([sys.executable, str(HERE / 'history-eval.py'), str(cards), '--repo', str(work), '--report', str(report),
                                '--developer-a', str(sum(1 for t in turns if t['role'] == 'A')), '--database', args.database], check=True,
                               stdout=subprocess.DEVNULL)
                results[name] = summarize_overlap(json.loads(report.read_text()))
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fixture', type=Path, default=HERE / 'fixtures' / 'summary-quality.json')
    parser.add_argument('--drafts', type=Path, required=True, help='JSON written by the ignored Rust test')
    parser.add_argument('--out', type=Path, required=True, help='where to write the result JSON (must not exist)')
    parser.add_argument('--database', default=os.environ.get('WK_TEST_DATABASE_URL'), help='disposable wk_test database; omit to skip the overlap part')
    args = parser.parse_args()
    turns = json.loads(args.fixture.read_text())['turns']
    drafts = json.loads(args.drafts.read_text())
    if [t['id'] for t in turns] != [d['id'] for d in drafts]:
        sys.exit('drafts do not match the fixture; regenerate them')
    result = {'text': score_text(turns, drafts)}
    if args.database:
        with tempfile.TemporaryDirectory() as work:
            result['overlap'] = run_overlap(turns, drafts, Path(work), args)
    fd = os.open(args.out, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as out:
        json.dump(result, out, indent=2)
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
