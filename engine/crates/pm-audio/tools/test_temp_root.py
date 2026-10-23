"""The campaign guard must detect leaks and remove its tree on every exit."""
import json
import importlib.util
import os
import signal
import time
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


class TempRootTests(unittest.TestCase):
    def test_permission_probe_requires_a_matching_process_group(self):
        spec = importlib.util.spec_from_file_location('audio_temp_guard', Path(__file__).with_name('temp-root.py'))
        guard = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(guard)
        with patch.object(guard.os, 'killpg', side_effect=PermissionError), \
                patch.object(guard.subprocess, 'check_output', return_value=' 17\n 23\n'):
            self.assertFalse(guard.group_exists(42))
            with self.assertRaises(PermissionError):
                guard.group_exists(23)

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

    def test_signal_joins_child_and_removes_tree(self):
        for stop in (signal.SIGTERM, signal.SIGINT):
            with tempfile.TemporaryDirectory(prefix='pm-audio-signal-test-') as output:
                report = Path(output) / 'report.json'
                process = subprocess.Popen([
                    sys.executable, str(Path(__file__).with_name('temp-root.py')),
                    '--report', str(report), '--', sys.executable, '-c',
                    'import os,time; open(os.environ["TMPDIR"]+"ready", "w").close(); time.sleep(100)',
                ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                try:
                    until = time.monotonic() + 5
                    ready = False
                    while time.monotonic() < until:
                        if report.exists():
                            try:
                                data = json.loads(report.read_text())
                                ready = (Path(data.get('tmpdir', '/nonexistent')) / 'ready').exists()
                            except json.JSONDecodeError:
                                pass
                            if ready:
                                break
                        time.sleep(.01)
                    self.assertTrue(ready, 'child did not become ready')
                    process.send_signal(stop)
                    self.assertNotEqual(process.wait(timeout=5), 0)
                    data = json.loads(report.read_text())
                    self.assertEqual(data['status'], 'failed')
                    self.assertTrue(data['removed'])
                    self.assertFalse(Path(data['tmpdir']).exists())
                    with self.assertRaises(ProcessLookupError):
                        os.kill(data['childPid'], 0)
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait()

    def test_signal_waits_for_separate_session_grandchild(self):
        with tempfile.TemporaryDirectory(prefix='pm-audio-grandchild-test-') as output:
            report = Path(output) / 'report.json'
            source = (
                'import os,signal,subprocess,sys,time\n'
                'child=subprocess.Popen([sys.executable,"-c","import time; time.sleep(100)"],start_new_session=True)\n'
                'def stop(signum,frame):\n'
                ' child.terminate(); child.wait(timeout=5); sys.exit(128+signum)\n'
                'signal.signal(signal.SIGTERM,stop)\n'
                'open(os.environ["TMPDIR"]+"ready","w").write(str(child.pid))\n'
                'time.sleep(100)\n'
            )
            process = subprocess.Popen([
                sys.executable, str(Path(__file__).with_name('temp-root.py')),
                '--report', str(report), '--', sys.executable, '-c', source,
            ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                until = time.monotonic() + 5
                ready = None
                while time.monotonic() < until:
                    if report.exists():
                        try:
                            data = json.loads(report.read_text())
                            ready = Path(data.get('tmpdir', '/nonexistent')) / 'ready'
                            if ready.exists():
                                break
                        except json.JSONDecodeError:
                            pass
                    time.sleep(.01)
                self.assertTrue(ready and ready.exists())
                grandchild = int(ready.read_text())
                process.terminate()
                self.assertNotEqual(process.wait(timeout=5), 0)
                data = json.loads(report.read_text())
                self.assertTrue(data['removed'])
                for pid in (data['childPid'], grandchild):
                    with self.assertRaises(ProcessLookupError):
                        os.kill(pid, 0)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()


if __name__ == '__main__':
    unittest.main()
