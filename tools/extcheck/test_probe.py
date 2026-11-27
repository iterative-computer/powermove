import copy
import unittest
import probe

class ProbeTests(unittest.TestCase):
    def recording(self):
        return {'activation': {'ok': True}, 'diagnostics': {'missing': []},
                'results': [{}, {'errors': [], 'time': 0}, [0,False], True]}

    def test_three_full_values_must_match_without_dropping_types_or_rows(self):
        first = self.recording()
        values = [copy.deepcopy(first) for _ in range(3)]
        self.assertEqual(probe.inspect_recordings(values), 'repeatable-component-probe')
        values[-1]['results'][1]['time'] = True
        self.assertEqual(probe.inspect_recordings(values), 'native-nondeterministic')
        with self.assertRaises(ValueError):probe.inspect_recordings(values[:2])

    def test_repeatable_activation_failures_and_missing_backends_are_not_success(self):
        values = [self.recording() for _ in range(3)]
        values[-1]['activation']['ok'] = False
        self.assertEqual(probe.inspect_recordings(values), 'native-activation-needs-backend')
        values[-1] = self.recording()
        values[-1]['diagnostics']['missing'] = ['synthetic-backend']
        self.assertEqual(probe.inspect_recordings(values), 'native-probe-needs-backend')
        values[-1] = self.recording()
        values[-1]['results'][1]['errors'] = [{}]
        self.assertEqual(probe.inspect_recordings(values), 'native-probe-needs-backend')

    def test_incomplete_native_reply_cannot_be_repeatable_component_evidence(self):
        values=[self.recording() for _ in range(3)]
        values[-1]['results']=[]
        self.assertEqual(probe.inspect_recordings(values),'native-probe-invalid')
