"""The summary-quality scorer must count what it claims to, using only the synthetic fixture."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('summary_quality_eval', ROOT / 'scripts/summary-quality-eval.py')
sq = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sq)

TURNS = [
    {'id': 't1', 'role': 'A', 'prompt': 'Cap retries', 'files': ['a.py'], 'topic': ['retr'], 'why': ['gateway', 'cap']},
    {'id': 't2', 'role': 'B', 'prompt': 'ok', 'files': [], 'topic': ['cache'], 'why': ['ttl']},
]


def card(title, rationale='', decisions=()):
    return {'title': title, 'intent': '', 'rationale': rationale, 'decisions': list(decisions), 'constraints': [], 'open_questions': []}


class ScoreTextTests(unittest.TestCase):
    def test_counts_titles_reasons_junk_and_held_turns(self):
        drafts = [
            {'id': 't1', 'rules': {'card': card('[auto-drafted] Cap retries', 'The gateway repeats charges. Hope this helps!')},
             'local': {'skipped': 'no endpoint'}, 'assistant': {'card': card('Retry cap', 'Gateway', ['cap at 3 attempts'])}},
            {'id': 't2', 'rules': {'card': card('[auto-drafted] ok', 'Added a cache')},
             'local': {'skipped': 'no endpoint'}, 'assistant': {'held': 'card requires private review'}},
        ]
        result = sq.score_text(TURNS, drafts)
        rules = result['rules']
        self.assertEqual((rules['cards'], rules['held']), (2, 0))
        self.assertEqual(rules['title_names_the_work'], 0.5)  # "Cap retries" yes, "ok" no
        self.assertEqual(rules['junk_title'], 0.5)             # the marker is not part of the title
        self.assertEqual(rules['has_pleasantry'], 0.5)
        self.assertEqual(rules['reasons_anywhere'], 0.5)       # 1.0 for t1 (gateway, cap), 0.0 for t2 (ttl)
        self.assertEqual(rules['has_decisions_or_constraints'], 0.0)
        self.assertEqual(result['assistant']['cards'], 1)
        self.assertEqual(result['assistant']['held'], 1)
        self.assertEqual(result['assistant']['reasons_anywhere'], 1.0)
        self.assertEqual(result['local']['cards'], 0)           # skipped counts as no card
        self.assertIsNone(result['local']['junk_title'])

    def test_junk_titles_are_recognised(self):
        for title in ['Hey! Quick one', 'yes go ahead', 'continue', 'Traceback (most recent call last):', "KeyError: 'x'", '```', 'Hi there']:
            self.assertTrue(sq.JUNK_TITLE.match(title), title)
        for title in ['Cap payment retries', 'Error handling for the gateway', 'Surefire config for the cache']:
            self.assertFalse(sq.JUNK_TITLE.match(title), title)


class CardLineTests(unittest.TestCase):
    drafts = [{'id': 't1', 'rules': {'card': dict(card('T'), capture_completeness='partial', evidence=[])}},
              {'id': 't2', 'rules': {'held': 'source requires private review'}}]

    def test_files_and_completeness_are_switchable_and_held_turns_have_no_card(self):
        lines = sq.card_lines(TURNS, self.drafts, 'rules', with_files=True, complete=False)
        self.assertEqual(lines[0]['card']['evidence'], [{'kind': 'file_observation', 'reference': 'a.py'}])
        self.assertEqual(lines[0]['card']['capture_completeness'], 'partial')
        self.assertNotIn('card', lines[1])
        self.assertEqual(lines[1]['error'], 'source requires private review')
        lines = sq.card_lines(TURNS, self.drafts, 'rules', with_files=False, complete=True)
        self.assertEqual(lines[0]['card']['evidence'], [])
        self.assertEqual(lines[0]['card']['capture_completeness'], 'complete')
        self.assertLess(lines[0]['observed_at_ms'], sq.card_lines(TURNS, self.drafts, 'rules', True, False)[1]['observed_at_ms'])


class SafetyTests(unittest.TestCase):
    def test_only_the_disposable_database_is_reset(self):
        with self.assertRaises(SystemExit):
            sq.reset_database('postgres://u@127.0.0.1:5432/wk_prod')

    def test_fixture_is_synthetic_and_complete(self):
        fixture = json.loads((ROOT / 'scripts/fixtures/summary-quality.json').read_text())
        self.assertIn('synthetic', fixture['note'].lower())
        for turn in fixture['turns']:
            for field in ('id', 'role', 'prompt', 'reply', 'files', 'topic', 'why', 'card'):
                self.assertIn(field, turn)
        roles = [t['role'] for t in fixture['turns']]
        self.assertEqual(roles, sorted(roles))  # history-eval treats the first N chats as developer A

    def test_mismatched_drafts_are_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            drafts = Path(temp) / 'd.json'
            drafts.write_text(json.dumps([{'id': 'other'}]))
            result = subprocess.run([sys.executable, str(ROOT / 'scripts/summary-quality-eval.py'), '--drafts', str(drafts),
                                     '--out', str(Path(temp) / 'o.json')], capture_output=True, text=True, env={'PATH': '/usr/bin:/bin'})
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('do not match', result.stderr)


if __name__ == '__main__':
    unittest.main()
