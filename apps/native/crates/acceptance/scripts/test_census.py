"""Reporting cannot drop rows, combine references, or conceal failing probes."""
import collections
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('census', Path(__file__).with_name('census.py'))
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)


def fixture():
    rows, policies = census.inventory()
    result = dict(reference=census.REFERENCES['main'], scope='full-inventory', cases=[],
                  compatibility=dict(reference=census.REFERENCES['v1.2.0'], scope='full-inventory', cases=[]))
    for row in rows:
        target = result if row['reference'] == 'main' else result['compatibility']
        target['cases'].append(dict(id=row['case_id'], status='blocked-by',
                                    blocked_by=row['blocked_by'], reason='Synthetic reporter test input'))
    return result, rows, policies


class CensusTests(unittest.TestCase):
    def test_exact_starting_population_has_one_reason_per_row(self):
        report, rows, policies = fixture()
        counts, table = census.assess(report)
        self.assertEqual(len(table), 5798)
        self.assertEqual(sum(v['blocked-by'] for v in counts.values()), 5798)
        self.assertEqual(sum(v['pass'] for v in counts.values()), 0)
        self.assertEqual({p['reason_class'] for p in policies.values()}, census.CLASSES)
        self.assertEqual(collections.Counter(r['reference'] for r in rows), {'main': 2662, 'v1.2.0': 3136})

    def test_real_failures_stay_failures_and_only_reported_passes_count(self):
        report, _, _ = fixture()
        candidates = [c for c in report['cases'] if c['id'] in {'MAIN.MCP.save_project.behavior', 'MAIN.MCP.list_projects.behavior'}]
        self.assertEqual(len(candidates), 2)
        for case, status in zip(candidates, ['pass', 'fail']):
            case.update(status=status, blocked_by=None, reason='Synthetic verdict propagation')
        counts, table = census.assess(report)
        self.assertEqual(counts[('main', 'portable-probe')], {'pass': 1, 'fail': 1, 'blocked-by': 17})
        self.assertEqual(len(table), 5798)

    def test_missing_or_duplicate_ids_are_refused(self):
        report, _, _ = fixture()
        missing = copy.deepcopy(report)
        missing['cases'].pop()
        with self.assertRaisesRegex(ValueError, 'disappeared'):
            census.assess(missing)
        report['cases'].append(report['cases'][0])
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            census.assess(report)

    def test_filtered_and_mixed_reference_reports_are_refused(self):
        report, _, _ = fixture()
        report['scope'] = 'filtered'
        with self.assertRaisesRegex(ValueError, 'full pinned-main'):
            census.assess(report)
        report['scope'] = 'full-inventory'
        report['compatibility']['reference'] = census.REFERENCES['main']
        with self.assertRaisesRegex(ValueError, 'separate legacy'):
            census.assess(report)

    def test_unconfigured_worker_and_new_failures_are_visible(self):
        report, _, _ = fixture()
        report['compatibility']['cases'].append(dict(id='unconfigured-worker', status='blocked-by', blocked_by='native-v8-backing', reason='worker missing'))
        report['cases'].append(dict(id='MAIN.file.utf16-ingress', status='fail', blocked_by=None, reason='real ingress failure'))
        counts, _ = census.assess(report)
        self.assertEqual(counts[('v1.2.0', 'additional-unconfigured-backing')]['blocked-by'], 1)
        self.assertEqual(counts[('main', 'additional-file-ingress-probe')]['fail'], 1)

    def test_invalid_status_backing_and_empty_evidence_are_refused(self):
        report, _, _ = fixture()
        for change, message in [(dict(status='skip'), 'unknown verdict'),
                                (dict(status='pass'), 'verdict and backing'),
                                (dict(reason=''), 'lacks evidence')]:
            bad = copy.deepcopy(report)
            bad['cases'][0].update(change)
            with self.assertRaisesRegex(ValueError, message):
                census.assess(bad)

    def test_fixture_is_reproducible_and_contains_no_external_runtime_inputs(self):
        path = census.DATA / 'fixtures/generate-portable.py'
        spec = importlib.util.spec_from_file_location('portable_fixture', path)
        generator = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(generator)
        self.assertEqual(generator.generate(), path.with_name('portable-media.pmv').read_bytes())


if __name__ == '__main__':
    unittest.main()
