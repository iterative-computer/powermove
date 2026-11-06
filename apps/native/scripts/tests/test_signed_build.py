"""Read-only command contracts, without claiming Apple code-signing proof."""
import copy
import importlib.util
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('signed_proof', Path(__file__).resolve().parents[1] / 'prove-signed-build.py')
proof = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(proof)


class SignedProof(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.relative = 'Contents/MacOS/native'
        self.manifest = {'signatures': {self.relative: {
            'team_id': 'SYNTHETIC0', 'identifier': 'synthetic.native',
            'requirement': 'identifier "synthetic.native"',
            'entitlements': {'com.apple.security.cs.allow-jit': True}}}}
        self.calls = []
        self.fail = None
        self.team = 'SYNTHETIC0'
        self.flags = '0x10000(runtime)'
        self.actual = {'com.apple.security.cs.allow-jit': True}
        self.changed = False
        self.audits = 0

    def audit(self, *args):
        self.audits += 1
        return {'issues': [], 'files': {'synthetic': {'sha256': str(self.changed and self.audits > 1)}},
                'macho': {self.relative: {'arm64': {}, 'x86_64': {}}}}

    def runner(self, args):
        self.calls.append(args)
        if self.fail and self.fail in args:
            raise subprocess.CalledProcessError(1, args)
        if '--entitlements' in args:
            return plistlib.dumps(self.actual).decode()
        if '--display' in args:
            return 'Identifier=synthetic.native\nTeamIdentifier=' + self.team + '\nCodeDirectory v=20500 size=1 flags=' + self.flags + '\n'
        return ''

    def result(self):
        return proof.prove(self.root, self.manifest, self.runner, self.audit)

    def test_all_slices_identity_entitlements_and_gatekeeper_are_inspected(self):
        self.assertEqual(self.result()['status'], 'passed')
        self.assertEqual(len([c for c in self.calls if '--entitlements' in c]), 2)
        self.assertTrue(any(c[0] == 'spctl' for c in self.calls))
        self.assertTrue(any(c[:3] == ['xcrun', 'stapler', 'validate'] for c in self.calls))
        self.assertFalse(any(c[0] in ('hdiutil', 'ditto') or '--sign' in c or 'submit' in c for c in self.calls))

    def test_wrong_team_and_missing_runtime_remain_blockers(self):
        self.team = 'WRONGTEAM0'
        self.flags = '0x0(none)'
        result = self.result()
        self.assertEqual(result['status'], 'blocked')
        self.assertFalse(any(s['verified'] for s in result['signatures'][self.relative].values()))

    def test_extra_entitlement_cannot_become_success(self):
        self.actual['com.apple.security.cs.disable-library-validation'] = True
        self.assertEqual(self.result()['status'], 'blocked')

    def test_missing_signature_inventory_remains_blocked(self):
        self.manifest['signatures'] = {}
        self.assertEqual(self.result()['status'], 'blocked')

    def test_failure_of_each_read_only_verifier_remains_visible(self):
        for token in ['--verify', 'spctl', 'stapler']:
            self.fail = token
            self.assertEqual(self.result()['status'], 'blocked')

    def test_candidate_mutation_during_proof_is_blocked(self):
        self.changed = True
        self.assertIn('bundle changed during signed proof', self.result()['issues'])

    def test_no_assumed_team_or_requirement_defaults(self):
        for field in ['team_id', 'identifier', 'requirement', 'entitlements']:
            original = copy.deepcopy(self.manifest)
            del self.manifest['signatures'][self.relative][field]
            with self.assertRaises(ValueError):
                self.result()
            self.manifest = original

    def test_malformed_entitlement_plist_is_refused(self):
        with self.assertRaises(ValueError):
            proof.entitlement_plist('not a plist')
