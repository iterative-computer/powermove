"""Safety and gate acceptance regressions; no app or private profile required."""
import math
import os
from pathlib import Path
import resource
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch, Mock
from types import SimpleNamespace

from budgets import ROWS, check, percentile, ratio
from macos import Processes, delta, host_seconds
import driver
from trace import analyze, load as load_trace


class GateTests(unittest.TestCase):
    def test_no_missing_row_can_pass(self):
        for row in ROWS:
            self.assertEqual(check(row, {}, {})['status'], 'INCOMPLETE', row)

    def test_cold_absolute_and_relative_are_both_required(self):
        self.assertEqual(check('cold_launch', {'latency_ms':[399]*30}, {'latency_ms':[800]*30})['status'], 'PASS')
        for n, e in [(400, 1000), (300, 500)]:
            self.assertEqual(check('cold_launch', {'latency_ms':[n]*30}, {'latency_ms':[e]*30})['status'], 'FAIL')

    def test_gpu_cpu_and_energy_cannot_be_substituted(self):
        result = check('playback_cost', {'cpu_percent':[1]*30}, {'cpu_percent':[10]*30})
        self.assertEqual(result['status'], 'INCOMPLETE')
        self.assertEqual(len(result['missing']), 2)

    def test_zero_drops_still_needs_headroom_and_full_duration(self):
        metrics = {'drops':[0]*30, 'frame_work_p95_ms':[8]*30, 'duration_s':[60]*30, 'fps':[60]*30}
        self.assertEqual(check('playback', metrics, metrics)['status'], 'PASS')
        self.assertEqual(check('playback', metrics, metrics, hz=120)['status'], 'PASS')
        for key, value in [('drops',1), ('frame_work_p95_ms',9), ('duration_s',59), ('fps',30)]:
            changed = dict(metrics, **{key:[value]*30})
            self.assertEqual(check('playback', changed, metrics)['status'], 'FAIL')
        self.assertEqual(check('playback', metrics, {})['status'], 'INCOMPLETE')

    def test_wrong_endpoint_or_machine_never_passes(self):
        metrics = {'latency_ms':[100]*30}
        for settings in [dict(endpoints=False), dict(conditions=False)]:
            self.assertEqual(check('cold_launch', metrics, {'latency_ms':[1000]*30}, **settings)['status'], 'INCOMPLETE')

    def test_tail_and_insufficient_runs_are_not_hidden(self):
        self.assertEqual(percentile([1]*28+[10,100], .95),10)
        self.assertEqual(check('cold_launch', {'latency_ms':[1]*29}, {'latency_ms':[100]*30})['status'], 'INCOMPLETE')
        self.assertIsNone(ratio(0, 0))
        for invalid in [[math.nan], [-1], [math.inf]]:
            with self.assertRaises(ValueError): percentile(invalid,.95)

    def test_memory_uses_absolute_saving_not_ratio(self):
        self.assertEqual(check('memory_small', {'footprint_mb':[200]*30}, {'footprint_mb':[300]*30})['status'], 'FAIL')
        self.assertEqual(check('memory_small', {'footprint_mb':[200]*30}, {'footprint_mb':[350]*30})['status'], 'PASS')

    def test_churned_process_family_is_rejected(self):
        with self.assertRaises(RuntimeError): delta({1:{}},{2:{}},1)

    def test_device_trace_never_fills_missing_timestamps_with_zero(self):
        result=analyze([
            {'gpu_ok':True,'display':10.003,'audio_host':10.001,'worker':10.,'completed':10.002,'input':9.999},
            {'gpu_ok':True,'display':None,'audio_host':10.},
            {'gpu_ok':False,'display':11.,'audio_host':11.},
            {'gpu_ok':True,'display':12.,'audio_host':None},
        ])
        self.assertEqual(result['positive_display_rows'],2)
        self.assertEqual(result['av_matched_samples'],1)
        self.assertAlmostEqual(result['av_mean_abs_offset_ms'],2.)
        self.assertAlmostEqual(result['input_to_display_ms']['p95'],4.)
        self.assertFalse(result['gate_eligible'])

    def test_nonfinite_or_negative_counter_can_never_pass(self):
        metrics={'drops':[0]*30, 'frame_work_p95_ms':[8]*30, 'duration_s':[60]*30, 'fps':[60]*30}
        for value in [float('nan'), float('inf'), -1]:
            changed=dict(metrics,drops=[value]*30)
            self.assertEqual(check('playback',changed,metrics)['status'],'INCOMPLETE')

    def test_lock_is_released_after_exception_and_other_owner_is_preserved(self):
        with tempfile.TemporaryDirectory() as root:
            lock=Path(root)/'gui.lock'
            def shell(_):
                with lock.open('x') as f: f.write('perf\n')
                return type('Result', (), {'returncode':0})()
            with patch.object(driver,'LOCK',lock), patch.object(driver,'command',shell):
                with self.assertRaisesRegex(RuntimeError,'exercise cleanup'):
                    with driver.gui_lock(): raise RuntimeError('exercise cleanup')
                self.assertFalse(lock.exists())
                with driver.gui_lock():
                    # Simulate another owner replacing a released lock (different inode).
                    replacement=Path(root)/'replacement'
                    replacement.write_text('other\n')
                    replacement.replace(lock)
                self.assertEqual(lock.read_text(),'other\n')

    def test_locked_preflight_returns_all_rows_without_starting_an_app(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as root:
            args=SimpleNamespace(output=Path(root)/'results.json', reference=Path('/tmp/unused'),
                                 refresh_hz=60, runs=30, playback_runs=3)
            blocked={'blockers':['Mac session is locked']}
            with patch.object(driver,'conditions',return_value=blocked), \
                 patch.object(driver,'machine',return_value={'required_rigs_certified':False}), \
                 patch.object(driver,'Processes'), patch.object(driver,'App') as app:
                result=driver.run(args)
                app.assert_not_called()
                self.assertEqual(len(result['results']),len(ROWS))
                self.assertTrue(all(r['status']=='INCOMPLETE' for r in result['results'].values()))
                self.assertTrue(args.output.exists())

    @unittest.skipUnless(os.uname().sysname == 'Darwin', 'Darwin counters')
    def test_mach_cpu_time_agrees_with_resource_usage(self):
        proc=Processes()
        before=proc.family(os.getpid())
        start=time.monotonic()
        a=resource.getrusage(resource.RUSAGE_SELF)
        while time.monotonic()-start < .15: pass
        b=resource.getrusage(resource.RUSAGE_SELF)
        observed=delta(before,proc.family(os.getpid()),time.monotonic()-start)
        expected=(b.ru_utime-a.ru_utime)+(b.ru_stime-a.ru_stime)
        self.assertAlmostEqual(observed['cpu_seconds'], expected, delta=.02)
        self.assertGreater(observed['family_phys_footprint_mb'],0)

    @unittest.skipUnless(os.uname().sysname == 'Darwin', 'Darwin counters')
    def test_family_includes_real_child_process(self):
        child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(10)'])
        try:
            family=Processes().family(os.getpid())
            self.assertIn(child.pid,family)
            self.assertIn(os.getpid(),family)
        finally:
            child.terminate()
            child.wait(timeout=3)

    def test_session_lock_requires_matching_owner_and_inode_and_stays_held(self):
        with tempfile.TemporaryDirectory() as root:
            lock = Path(root) / 'gui.lock'
            lock.write_text('onscreen\n')
            with patch.object(driver, 'LOCK', lock):
                with driver.gui_lock(session_inode=lock.stat().st_ino):
                    self.assertTrue(lock.exists())
                self.assertTrue(lock.exists())
                with self.assertRaises(RuntimeError):
                    with driver.gui_lock(session_inode=lock.stat().st_ino + 1): pass
                lock.write_text('foreign\n')
                with self.assertRaises(RuntimeError):
                    with driver.gui_lock(session_inode=lock.stat().st_ino): pass

    def test_diagnostic_exceptions_never_allow_locked_disk_power_or_build_failures(self):
        conditions = {'blockers': ['One-minute load is not below 12',
            'Other Powermove/Electron instances are active; leave them untouched']}
        self.assertEqual(driver.execution_blockers(conditions, True), [])
        self.assertEqual(driver.execution_blockers(conditions, False), conditions['blockers'])
        for reason in ['Mac session is locked', 'Less than 8 GiB free',
                       'Machine is not on AC power', 'Build processes are active; machine is not settled']:
            self.assertEqual(driver.execution_blockers({'blockers': [reason]}, True), [reason])

    def test_invalid_pairs_are_only_summarized_separately_and_never_pass(self):
        pair = {'valid_conditions': False,
                'native': {'idle': {'cpu_percent': .1}},
                'electron': {'idle': {'cpu_percent': 1.}}}
        data = {'pairs': [pair] * 30, 'refresh_hz': 120,
                'machine': {'required_rigs_certified': False}}
        self.assertEqual(driver.summary(data)['idle']['stats']['native'], {})
        diagnostic = driver.summary(data, diagnostic=True)['idle']
        self.assertEqual(diagnostic['stats']['native']['cpu_percent']['n'], 30)
        self.assertEqual(diagnostic['status'], 'INCOMPLETE')

    def test_service_ndjson_and_single_row_preserve_missing_presentations(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'trace.json'
            path.write_text('{"gpu_ok":true,"display":1.0}\n{"gpu_ok":true,"display":null}\n')
            report = load_trace(path)
            self.assertEqual(report['rows'], 2)
            self.assertEqual(report['positive_display_rows'], 1)
            path.write_text('{"gpu_ok":true,"display":null}\n')
            self.assertEqual(load_trace(path)['positive_display_rows'], 0)
            path.write_text('{"gpu_ok":true}\nmalformed\n')
            with self.assertRaises(ValueError): load_trace(path)

    def test_setup_seek_skips_only_an_already_displayed_native_frame(self):
        app = Mock(kind='native')
        app.request.return_value = {'frame': {'ticks': 0}}
        driver.position(app, 0.)
        app.action.assert_not_called()
        driver.position(app, 1 / 30)
        app.action.assert_called_once_with('seek', seconds=1 / 30)
        app.kind = 'electron'
        driver.position(app, 0.)
        self.assertEqual(app.action.call_count, 2)

    def test_app_local_copies_keep_fixture_immutable_and_partial_values_survive(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            source = root / 'corpus'; source.mkdir()
            (source / 'document.json').write_text('{"fixture":true}')
            scratch = root / 'scratch'; scratch.mkdir()
            args = SimpleNamespace(output=root/'results.json',corpus=source,
                fps=30,settle_seconds=.001,idle_seconds=.001,playback_runs=1)
            paths = []
            def factory(kind, args, scratch):
                app = Mock(kind=kind, pid=123, ready_ms=1, ready={'endpoint':'test'},
                           trace_path=root/'absent.trace')
                def action(op, **kwargs):
                    if op == 'open':
                        path=Path(kwargs['path']); paths.append(path)
                        path.write_text('PMV4 mutated disposable app copy')
                    return {'ms':1,'cache_hit':True}
                app.action.side_effect = action
                def request(op):
                    if op == 'play': raise RuntimeError('forced playback failure')
                    return {'frame':{'ticks':0}}
                app.request.side_effect = request
                return app
            proc=Mock();proc.family.return_value={123:{'phys_footprint':1000000}}
            with patch.object(driver,'App',side_effect=factory), patch.object(driver,'observe',return_value={'family_phys_footprint_mb':1,'per_process':[]}):
                for kind in ['native','electron']:
                    values,errors=driver.one_app(kind,args,proc,scratch,0)
                    self.assertIn('cold_launch',values)
                    self.assertIn('open',values)
                    self.assertIn('memory_small',values)
                    self.assertIn('playback_cost',errors)
            self.assertNotEqual(paths[0],paths[1])
            self.assertEqual((source/'document.json').read_text(),'{"fixture":true}')

    @unittest.skipUnless(os.uname().sysname == 'Darwin', 'Darwin host clock')
    def test_foreground_clock_matches_native_mach_host_origin(self):
        import ctypes
        host_seconds()  # Load Quartz before the bounded comparison.
        lib=ctypes.CDLL('/usr/lib/libSystem.B.dylib')
        lib.mach_absolute_time.restype=ctypes.c_uint64
        proc=Processes()
        native=lib.mach_absolute_time()*proc.timebase[0]/proc.timebase[1]/1e9
        self.assertAlmostEqual(host_seconds(),native,delta=.05)


if __name__ == '__main__':
    unittest.main()
