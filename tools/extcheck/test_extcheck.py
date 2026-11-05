import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch, Mock, call
import run
import gui_capture


def trace(name="services.get", result=1):
    rows=[{"type":"header","format":"powermove.behavior","version":1},
          {"seq":1,"type":"call","call":1,"parent":None,"name":name,"args":[]},
          {"seq":2,"type":"return","call":1,"value":result},
          {"seq":3,"type":"end","valid":True,"calls":1}]
    return "\n".join(json.dumps(row) for row in rows)+"\n"


class HarnessTests(unittest.TestCase):
    def setUp(self):
        active=patch.object(run,'USER_ACTIVE',Mock(exists=Mock(return_value=False)))
        active.start()
        self.addCleanup(active.stop)
        disk=patch("gui_capture.disk_ready",return_value=True)
        disk.start()
        self.addCleanup(disk.stop)

    def test_owner_pause_waits_five_minutes_without_starting_work(self):
        pause=Mock()
        pause.exists.side_effect=[True,False]
        with patch.object(run,'PAUSE_LOCAL',pause),patch('run.time.sleep') as sleep,patch('builtins.print'):
            run.wait_for_local_execution()
        sleep.assert_called_once_with(300)

    def test_owner_pause_stops_only_the_owned_capture_group(self):
        import signal
        child=Mock(pid=4242)
        pause=Mock()
        pause.exists.side_effect=[False,True]
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as temporary,patch('run.disk_ready',return_value=True), \
             patch('run.subprocess.Popen',return_value=child),patch.object(run,'PAUSE_LOCAL',pause), \
             patch('run.os.killpg') as kill:
            with self.assertRaises(run.LocalPause):
                run.capture_unlocked(['synthetic-driver'],{'role':'native'},Path(temporary)/'out',1)
        kill.assert_called_once_with(4242,signal.SIGTERM)
        child.wait.assert_called_once_with(timeout=1)
        child.communicate.assert_not_called()

    def test_disk_pressure_waits_and_resumes_at_the_revised_floor(self):
        from types import SimpleNamespace
        below=False
        ready=True
        with patch('run.wait_for_local_execution'),patch('run.disk_ready',side_effect=[below,ready]), \
             patch('run.time.sleep') as sleep, patch('builtins.print'):
            run.disk_check()
        sleep.assert_called_once_with(120)

    def test_standalone_gui_guard_stops_the_owned_app_on_pause(self):
        import signal
        child=Mock(pid=4242)
        child.poll.return_value=0
        with patch('gui_capture.gui_unavailable',side_effect=[False,True]), \
             patch('gui_capture.subprocess.Popen',return_value=child),patch('gui_capture.os.killpg') as kill:
            self.assertEqual(gui_capture.guarded_run(['synthetic-driver']),77)
        kill.assert_called_once_with(4242,signal.SIGTERM)

    def test_user_active_refuses_standalone_app_launch(self):
        with patch('gui_capture.Path.exists',side_effect=[False,True]), \
             patch('gui_capture.subprocess.Popen') as launch:
            self.assertEqual(gui_capture.guarded_run(['synthetic-driver']),77)
        launch.assert_not_called()

    def test_user_active_prevents_either_capture_role_from_launching(self):
        with tempfile.TemporaryDirectory() as temporary, \
             patch.object(run,'USER_ACTIVE',Mock(exists=Mock(return_value=True))), \
             patch('run.subprocess.Popen') as launch:
            for role in ['reference','native']:
                with self.assertRaises(run.LocalPause):
                    run.capture_unlocked(['synthetic-driver'],{'role':role},Path(temporary)/role,1)
        launch.assert_not_called()

    def test_stalled_driver_gets_a_second_graceful_signal_for_detached_app_cleanup(self):
        import signal,subprocess
        child=Mock(pid=4242)
        child.wait.side_effect=[subprocess.TimeoutExpired('synthetic',1),0]
        with patch('gui_capture.os.killpg') as kill:
            gui_capture.stop_owned_process(child)
        self.assertEqual(kill.call_args_list,[call(4242,signal.SIGTERM),call(4242,signal.SIGTERM)])

    def test_unresponsive_owned_driver_is_killed_after_both_cleanup_attempts(self):
        import signal,subprocess
        child=Mock(pid=4242)
        child.wait.side_effect=[subprocess.TimeoutExpired('synthetic',1),
                                subprocess.TimeoutExpired('synthetic',1),-signal.SIGKILL]
        with patch('gui_capture.os.killpg') as kill:
            gui_capture.stop_owned_process(child)
        self.assertEqual(kill.call_args_list,[call(4242,signal.SIGTERM),
                                             call(4242,signal.SIGTERM),call(4242,signal.SIGKILL)])

    def test_busy_gui_retries_before_starting_capture_deadline(self):
        import fcntl
        with tempfile.TemporaryFile() as lock, \
             patch('gui_capture.open',return_value=lock), \
             patch('gui_capture.Path.exists',return_value=False), \
             patch('gui_capture.fcntl.lockf',side_effect=[BlockingIOError(),None,None]) as acquire, \
             patch('gui_capture.time.sleep') as sleep, \
             patch('gui_capture.subprocess.check_output',return_value='unlocked\n'):
            with gui_capture.gui_slot():
                sleep.assert_called_once_with(60)
            self.assertEqual(acquire.call_count,3)
            self.assertEqual(acquire.call_args.args[1],fcntl.LOCK_UN)

    def test_both_capture_roles_lease_the_gui_before_starting_the_driver(self):
        from contextlib import contextmanager
        events=[]
        @contextmanager
        def lease():
            events.append('acquire')
            try:
                yield
            finally:
                events.append('release')
        def capture(_command,request,_output,_timeout):
            events.append(request['role'])
            return {'status':'synthetic'}
        with patch('run.disk_check'),patch('gui_capture.gui_slot',lease), \
             patch('run.sys.platform','darwin'), patch('run.capture_unlocked',side_effect=capture):
            for role in ['reference','native']:
                run.capture(['synthetic-driver'],{'role':role},Path('/unused'),1)
        self.assertEqual(events,['acquire','reference','release','acquire','native','release'])

    def test_paused_capture_releases_lease_and_retries_only_its_owned_output(self):
        from contextlib import contextmanager
        events=[]
        @contextmanager
        def lease():
            events.append('acquire')
            try:
                yield
            finally:
                events.append('release')
        attempts=0
        def capture(_command,_request,output,_timeout):
            nonlocal attempts
            attempts+=1
            output.mkdir()
            if attempts==1:
                (output/'partial').write_text('synthetic')
                raise run.LocalPause()
            self.assertFalse((output/'partial').exists())
            return {'status':'synthetic'}
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            sibling=root/'untouched'
            sibling.write_text('synthetic')
            with patch('run.disk_check'),patch('gui_capture.gui_slot',lease), \
                 patch('run.sys.platform','darwin'), patch('run.capture_unlocked',side_effect=capture):
                self.assertEqual(run.capture(['synthetic-driver'],{'role':'native'},root/'out',1),
                                 {'status':'synthetic'})
            self.assertEqual(sibling.read_text(),'synthetic')
        self.assertEqual(attempts,2)
        self.assertEqual(events,['acquire','release','acquire','release'])

    def test_trace_rejects_missing_end_sequence_gaps_and_wrong_parent(self):
        for invalid in [trace().rsplit('\n',2)[0],trace().replace('"seq": 2','"seq": 9'),
                        trace().replace('"parent": null','"parent": 1'),trace().replace('"valid": true','"valid": false')]:
            with self.assertRaises(run.Blocked): run.validate_trace(invalid)
        self.assertEqual(len(run.validate_trace(trace())),4)

    def test_an_early_end_cannot_hide_extra_rows(self):
        text=trace()+json.dumps({"seq":4,"type":"end","valid":True,"calls":1})+'\n'
        with self.assertRaises(run.Blocked):run.validate_trace(text)

    def test_trace_equality_preserves_js_types_and_property_order(self):
        self.assertNotEqual(run.exact(True),run.exact(1))
        self.assertNotEqual(run.exact({'a':1,'b':2}),run.exact({'b':2,'a':1}))
        with self.assertRaises(run.Blocked):run.validate_trace(trace().replace('"seq": 1','"seq": true'))

    def test_readonly_input_copy_and_symlink_escape(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); source=root/'source'; source.mkdir()
            (source/'manifest.json').write_text('{"id":"synthetic"}')
            original=(source/'manifest.json').read_bytes()
            run.copy_extension(source,root/'copy')
            (root/'copy/manifest.json').write_text('changed')
            self.assertEqual((source/'manifest.json').read_bytes(),original)
            (source/'escape').symlink_to(root)
            with self.assertRaises(run.Blocked):run.copy_extension(source,root/'refused')

    def test_artifact_paths_cannot_escape_temporary_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for value in ['../private.png','/tmp/private.png']:
                with self.assertRaises(run.Blocked):run.safe_file(root,value)

    def test_all_required_mismatch_families(self):
        names=['PM.model.add','services.get','render.raster','canvas.fillRect','dom.input','lifecycle.dispose']
        self.assertEqual([run.family(name) for name in names],list(run.FAMILIES))

    def test_visual_oracle_decodes_pixels_instead_of_comparing_png_encoding(self):
        from PIL import Image
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);a=root/'a.png';b=root/'b.png'
            Image.new('RGBA',(2,2),(255,0,0,0)).save(a,compress_level=0)
            Image.new('RGBA',(2,2),(0,255,0,0)).save(b,compress_level=9)
            self.assertTrue(run.visual_diff(a,b)['equal'])
            Image.new('RGBA',(2,2),(255,0,0,255)).save(b)
            self.assertFalse(run.visual_diff(a,b)['equal'])
            Image.new('RGBA',(3,2)).save(b)
            self.assertTrue(run.visual_diff(a,b)['size_mismatch'])

    def test_behavior_difference_is_reported_without_private_values(self):
        from PIL import Image
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);a=root/'a';b=root/'b';a.mkdir();b.mkdir()
            for folder,result in [(a,'private-reference-value'),(b,'private-native-value')]:
                (folder/'trace.jsonl').write_text(trace(result=result))
                (folder/'interactions.json').write_text('[{"op":"screenshot","result":null}]')
                Image.new('RGBA',(2,2)).save(folder/'checkpoint.png')
            script={'steps':[{'op':'screenshot','label':'checkpoint'}]}
            report=run.compare(a,b,script)
            self.assertEqual(report,[{'kind':'trace','row':2,'family':'sync services'}])
            self.assertNotIn('private',json.dumps(report))

    def test_missing_capture_and_unapproved_block_reason_cannot_pass(self):
        with self.assertRaisesRegex(run.Blocked,'native-integrated-runner-unavailable'):
            run.capture(None,{},Path('/unused'),1)

    def test_script_cannot_claim_completion_without_visual_evidence(self):
        for script in [{'version':1,'complete':True,'steps':[{'op':'reload'}]},
                       {'version':1,'complete':True,'steps':[{'op':'screenshot','label':'../escape'}]}]:
            with self.assertRaises(run.Blocked):run.validate_script(script)
        run.validate_script(run.DEFAULT_SCRIPT)

    def test_population_never_drops_absent_inputs_or_catalog_failures(self):
        counts = [run.population_counts(source, 0, False) for source in run.SOURCES]
        self.assertEqual(sum(row['total'] for row in counts), 58)
        self.assertEqual(sum(row['cant_run'] for row in counts), 58)
        local = run.population_counts('local-corpus', 9, True)
        self.assertEqual((local['total'], local['cant_run'], local['observed']), (10, 1, 9))
        extra = run.population_counts('store', 5, True)
        self.assertEqual((extra['total'], extra['unavailable']), (5, 0))

    def test_linux_full_capture_reports_a_blocker_without_a_gui_probe(self):
        with patch('run.sys.platform', 'linux'), patch('gui_capture.gui_slot') as gui:
            with self.assertRaisesRegex(run.Blocked, 'macos-capture-unavailable'):
                run.capture(['synthetic-driver'], {}, Path('/unused'), 1)
        gui.assert_not_called()

    def test_script_clock_rejects_invalid_and_unbounded_progression(self):
        for value in [True, None, -1, 60_001, float('nan'), float('inf')]:
            script = {**run.DEFAULT_SCRIPT, 'steps': [{'op': 'advance', 'ms': value},
                      {'op': 'screenshot', 'label': 'active'}]}
            with self.assertRaises(run.Blocked):run.validate_script(script)
        run.validate_script({**run.DEFAULT_SCRIPT, 'steps': [{'op':'advance','ms':16},
                            {'op':'screenshot','label':'active'}]})

    def test_absent_populations_still_produce_a_58_case_open_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            output=Path(temporary)/'summary.json'
            def discover(path):
                if path.name == 'unavailable-owner':raise run.Blocked('input-root-missing')
                return []
            with patch('sys.argv',['extcheck','--reference-root',temporary,
                       '--owner-root',str(Path(temporary)/'unavailable-owner'),
                       '--summary',str(output),'--store']), patch('run.disk_check'), \
                 patch('run.discover',side_effect=discover), \
                 patch('run.PublicStore.catalog',side_effect=run.Blocked('store-public-read-failed')), \
                 patch('run.fixture_identity',return_value={'version':1,'sha256':'synthetic'}), \
                 patch('builtins.print'):
                self.assertEqual(run.main(),2)
            result=json.loads(output.read_text())
            self.assertEqual(result['gate'],'OPEN')
            self.assertEqual(sum(row['total'] for row in result['sources'].values()),58)
            self.assertEqual(sum(row['cant_run'] for row in result['sources'].values()),58)
            self.assertFalse(result['sources']['store']['enumerated'])

    def test_capture_requires_the_exact_fixture_identity(self):
        fixture={'version':1,'sha256':'synthetic'}
        request={'role':'native','case':'local-corpus-0001','input_sha256':'input',
                 'script_sha256':'script','clock_fixture':fixture}
        for actual in [None,{'version':1,'sha256':'different'},fixture]:
            with tempfile.TemporaryDirectory() as temporary:
                output=Path(temporary)/'out'
                status={'status':'captured',**{key:request[key] for key in ('case','input_sha256','script_sha256')},
                        'clock_fixture':actual,'path':{'logic':'runtime-v8','panels':'webkit-remote-dom','canvas':'native-recorder'}}
                child=Mock(returncode=0)
                child.poll.return_value=0
                child.communicate.side_effect=lambda *_args,**_kwargs:(output/'capture.json').write_text(json.dumps(status))
                with patch('run.subprocess.Popen',return_value=child),patch('run.disk_ready',return_value=True), \
                     patch.object(run,'PAUSE_LOCAL',Mock(exists=Mock(return_value=False))):
                    if actual==fixture:
                        self.assertEqual(run.capture_unlocked(['synthetic-driver'],request,output,1),status)
                    else:
                        with self.assertRaisesRegex(run.Blocked,'capture-clock-fixture-mismatch'):
                            run.capture_unlocked(['synthetic-driver'],request,output,1)

    def test_unstable_second_recording_does_not_hide_the_third_or_the_other_host(self):
        from PIL import Image
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);source=root/'source';source.mkdir()
            (source/'manifest.json').write_text('{"id":"synthetic"}')
            output=root/'summary.json'
            def discover(path):return [source] if path.name=='corpus' else []
            def capture(_command,request,output,_timeout):
                output.mkdir()
                value=2 if output.name.endswith('repeat-1') else 1
                (output/'trace.jsonl').write_text(trace(result=value))
                (output/'interactions.json').write_text(json.dumps([
                    {'op':step['op'],'result':None} for step in run.DEFAULT_SCRIPT['steps']]))
                Image.new('RGBA',(2,2)).save(output/'active.png')
            with patch('sys.argv',['extcheck','--reference-root',temporary,'--owner-root',str(root/'owner'),
                       '--corpus-root',str(root/'corpus'),'--summary',str(output),
                       '--native-command','["synthetic-driver"]']),patch('run.disk_check'), \
                 patch('run.discover',side_effect=discover),patch('run.capture',side_effect=capture) as captures, \
                 patch('run.fixture_identity',return_value={'version':1,'sha256':'synthetic'}),patch('builtins.print'):
                self.assertEqual(run.main(),2)
            self.assertEqual(captures.call_count,6)
            roles=[item.args[1]['role'] for item in captures.call_args_list]
            self.assertEqual(roles,['reference']*3+['native']*3)
            result=json.loads(output.read_text())
            self.assertEqual(result['reasons']['reference-nondeterministic'],1)
            self.assertEqual(result['reasons']['native-nondeterministic'],1)
            self.assertEqual(sum(row['cant_run'] for row in result['sources'].values()),58)

    def test_repeat_and_paired_drivers_load_the_same_private_scenario(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            script={**run.DEFAULT_SCRIPT,'steps':[{'op':'advance','ms':16},{'op':'screenshot','label':'authored'}]}
            (root/'local-corpus-0001.json').write_text(json.dumps(script))
            self.assertEqual(run.load_script(root,'local-corpus-0001'),script)
            self.assertIs(run.load_script(root,'local-corpus-0002'),run.DEFAULT_SCRIPT)
            (root/'local-corpus-0001.json').write_text('[]')
            with self.assertRaisesRegex(run.Blocked,'invalid-script'):run.load_script(root,'local-corpus-0001')


if __name__=='__main__':unittest.main()
