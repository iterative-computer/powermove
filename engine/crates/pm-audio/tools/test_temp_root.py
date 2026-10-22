"""The campaign guard must detect leaks and remove its tree on every exit."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class TempRootTests(unittest.TestCase):
    def run_child(self, source):
        with tempfile.TemporaryDirectory(prefix='pm-audio-guard-test-') as output:
            report = Path(output) / 'report.json'
            result = subprocess.run([sys.executable, str(Path(__file__).with_name('temp-root.py')),
                                     '--report', str(report), '--', sys.executable, '-c', source],
                                    capture_output=True, text=True)
            data = json.loads(report.read_text())
        self.assertTrue(data['removed'])
        self.assertFalse(Path(data['tmpdir']).exists())
        return result.returncode, data

    def test_success_and_exception_clean_up(self):
        for exit_code in (0, 7):
            code, data = self.run_child(
                'import tempfile,sys\n'
                'with tempfile.TemporaryDirectory() as root:\n'
                ' open(root+"/audio.wav", "wb").write(b"audio")\n'
                f' sys.exit({exit_code})\n')
            self.assertEqual(code, exit_code)
            self.assertTrue(data['noGrowth'])
            self.assertEqual(data['status'], 'passed' if exit_code == 0 else 'failed')

    def test_leak_fails_even_when_child_succeeds(self):
        code, data = self.run_child(
            'import tempfile\n'
            'root=tempfile.mkdtemp()\n'
            'open(root+"/audio.wav", "wb").write(b"audio")\n')
        self.assertNotEqual(code, 0)
        self.assertFalse(data['noGrowth'])
        self.assertGreater(data['after']['bytes'], data['before']['bytes'])
        self.assertEqual(data['status'], 'failed')


if __name__ == '__main__':
    unittest.main()
