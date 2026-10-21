"""The support table is generated from compatibility.json, and release drift is detected offline."""
import contextlib, importlib.util, io, json, unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), ROOT / 'scripts' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SupportTableTests(unittest.TestCase):
    def test_committed_table_matches_the_source(self):
        self.assertEqual(load('support-table').main(['--check']), 0)

    def test_every_adapter_states_its_boundary(self):
        data = json.loads((ROOT / 'docs/adapters/compatibility.json').read_text())
        required = load('support-table').REQUIRED
        for adapter in data['adapters']:
            for key in required:
                self.assertIn(key, adapter, adapter['client'])
        clients = [a['client'] for a in data['adapters']]
        self.assertIn('Windows (any client)', clients)
        self.assertIn('Any other client or version', clients)

    def test_documents_link_to_the_single_table(self):
        for path in ('README.md', 'docs/adapters/setup.md', 'docs/architecture/completion-matrix.md'):
            self.assertIn('support.md', (ROOT / path).read_text(), path)

    def test_missing_field_is_reported_not_rendered(self):
        with self.assertRaises(SystemExit):
            load('support-table').render({'adapters': [{'client': 'X'}]})


class ReleaseCheckTests(unittest.TestCase):
    def run_check(self, **latest):
        module = load('check-client-releases')
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = module.main([arg for k, v in latest.items() for arg in ('--latest', '%s=%s' % (k.replace('_', ' '), v))])
        return code, out.getvalue()

    def test_tested_versions_report_no_drift(self):
        code, text = self.run_check(Cursor='3.23.23', Claude_Code='2.1.293')
        self.assertEqual(code, 0, text)

    def test_newer_release_is_flagged_with_the_next_step(self):
        code, text = self.run_check(Cursor='3.24.0', Claude_Code='2.1.293')
        self.assertEqual(code, 1)
        self.assertIn('Cursor: 3.24.0 is out', text)
        self.assertIn('support-table.py', text)

    def test_version_comparison_is_numeric(self):
        key = load('check-client-releases').version_key
        self.assertGreater(key('2.1.306'), key('2.1.293'))
        self.assertGreater(key('2.10.0'), key('2.9.9'))


if __name__ == '__main__':
    unittest.main()
