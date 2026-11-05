"""Synthetic privacy, source immutability and worker-timeout regressions."""
import importlib.util
from pathlib import Path
import os
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('corpus', Path(__file__).with_name('run.py'))
corpus = importlib.util.module_from_spec(spec)
spec.loader.exec_module(corpus)

class ContainerCapacity(unittest.TestCase):
    def test_macos_capacity_uses_exact_apfs_container_bytes(self):
        output = SimpleNamespace(stdout='Container Free Space: 10.7 GB (10737418240 Bytes)\n')
        with patch.object(corpus.sys, 'platform', 'darwin'), patch.object(corpus.subprocess, 'run', return_value=output) as run, patch.object(corpus.shutil, 'disk_usage', side_effect=AssertionError('volume free space queried')):
            self.assertEqual(corpus.free_bytes(), 10737418240)
        self.assertEqual(run.call_args.args[0], ['diskutil', 'info', '/'])

    def test_missing_container_measurement_refuses_admission(self):
        with patch.object(corpus.sys, 'platform', 'darwin'), patch.object(corpus.subprocess, 'run', return_value=SimpleNamespace(stdout='Volume Free Space: 100 GB\n')):
            with self.assertRaisesRegex(RuntimeError, 'container_free_space_unavailable'):
                corpus.free_bytes()


class CorpusSafety(unittest.TestCase):
    def setUp(self):
        # Unit fixtures are only a few bytes. Model capacity explicitly so
        # another lane's builds cannot change a timeout/pause test's meaning.
        capacity=patch.object(corpus,'free_bytes',return_value=corpus.FLOOR+1024**3)
        capacity.start()
        self.addCleanup(capacity.stop)

    def test_frozen_worker_and_export_child_use_frozen_compiler(self):
        import sys
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory)
            worker,helper=base/'worker',base/'pm-glsl-compiler'
            helper.write_text('#!/usr/bin/env python3\nprint("synthetic compiler")\n')
            # Mimic a worker's export child inheriting the compiler environment.
            child='import os,subprocess;subprocess.run([os.environ["POWERMOVE_GLSL_COMPILER"]],check=True)'
            worker.write_text('#!/usr/bin/env python3\nimport subprocess,sys\nsubprocess.run([sys.executable,"-c",'+repr(child)+'],check=True)\n')
            worker.chmod(0o700)
            helper.chmod(0o700)
            frozen_root=base/'frozen'
            frozen_root.mkdir()
            frozen,compiler,identities=corpus.freeze_tools(worker,helper,frozen_root)
            worker.unlink()
            helper.unlink()
            log=base/'private.log'
            self.assertEqual(corpus.call(frozen,[],log,timeout=5,compiler=compiler),0)
            self.assertEqual(log.read_text().strip(),'synthetic compiler')
            self.assertEqual(corpus.digest(compiler),identities['compiler_sha256'])

    def test_frozen_tools_refuse_modified_bytes_and_symlinks(self):
        for symlink in (False, True):
            with tempfile.TemporaryDirectory() as directory:
                base = Path(directory)
                worker, helper = base/'worker', base/'helper'
                for p in (worker, helper):
                    p.write_text('#!/bin/sh\nexit 0\n')
                    p.chmod(0o700)
                root = base/'frozen'
                root.mkdir()
                frozen, compiler, _ = corpus.freeze_tools(worker, helper, root)
                compiler.unlink()
                if symlink:
                    compiler.symlink_to(helper)
                else:
                    compiler.write_bytes(b'changed')
                    compiler.chmod(0o700)
                with self.assertRaisesRegex(RuntimeError, 'frozen_tool_changed'):
                    corpus.freeze_tools(worker, helper, root)

    def test_selection_prefers_exact_content_key_then_bare_id(self):
        records = [{'key':'clip','revision':'r2','source':'stale','snapshot':'db'},
                   {'key':'media:current','revision':'r2','source':'current'}]
        asset = {'fingerprint':'current','revision':'r2'}
        self.assertEqual(corpus.select_records('clip', asset, records, Path('.')), [records[1]])
        self.assertEqual(corpus.select_records('clip', asset, records[:1], Path('.')), records[:1])
        asset['revision'] = 'new'
        self.assertEqual(corpus.select_records('clip', asset, records, Path('.')), [])

    def test_content_object_without_key_manifest_is_captured_for_verification(self):
        h = 'a' * 64
        selected = corpus.select_records('clip', {'contentHash':h}, [], Path('profile'))
        self.assertEqual(selected[0]['source'], str(Path('profile/Media Store/objects/aa') / h))
        self.assertEqual(selected[0]['blake3'], h)
        for h in ('../../untrusted', 'a'*63, 'z'*64):
            self.assertEqual(corpus.select_records('clip', {'contentHash':h}, [], Path('.')), [])

    def test_captured_source_link_must_match_the_imported_checksum(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'private-copy'
            path.write_bytes(b'original imported bytes')
            expected = corpus.digest(path)
            record = {'key':'synthetic', 'value':{'sourcePath':'/synthetic/original', 'sha256':expected}}
            selected = corpus.captured_record(record, str(path))
            self.assertEqual(selected['sha256'], expected)
            self.assertEqual(selected['value'], record['value'])
            path.write_bytes(b'externally changed bytes')
            with self.assertRaisesRegex(corpus.BatchLimit, 'media_record_checksum_mismatch'):
                corpus.captured_record(record, str(path))
            for invalid in (expected[:16], 'z'*64, 123, None):
                with self.assertRaises(corpus.BatchLimit):
                    corpus.captured_record({'value':{'sha256':invalid}}, str(path))

    def test_changed_source_cannot_reenter_through_another_assets_link(self):
        import json, hashlib
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            profile, target = base/'profile', base/'snapshot'
            (profile/'store').mkdir(parents=True)
            index = profile/'Media Store/Index'
            index.mkdir(parents=True)
            source = base/'source.png'
            source.write_bytes(b'changed source bytes')
            record = {'key':'media:synthetic','sourcePath':str(source),
                      'sha256':hashlib.sha256(b'imported source bytes').hexdigest()}
            (index/'synthetic.json').write_text(json.dumps(record))
            args = SimpleNamespace(profile=profile, projects_only=False, projects=[], extra_files=None,
                resume_snapshot=False, resume_results=False, report=base/'report.json',
                retain_for_debug=False, batch_gib=1.5, only_ids=None, binary=base/'worker', compiler=base/'helper', timeout=5)
            def worker(binary, arguments, log, **kwargs):
                if arguments[0] == 'inventory':
                    row = {'id':'synthetic','library':'synthetic','assets':{
                        'old':{'storageKey':record['key'],'sourcePath':str(source)},
                        'fresh':{'sourcePath':str(source)}}}
                    (target/'inventory.json').write_text(json.dumps({'projects':[row], 'associated_files':[]}))
                elif len(arguments)>1 and arguments[1] == '--clone-one':
                    corpus.clone(Path(arguments[2]), Path(arguments[3]))
                else:
                    self.assertEqual(arguments[0], 'project')
                    manifest = json.loads((target/'media.json').read_text())
                    self.assertEqual(manifest, {'records':[], 'linked':{}})
                    (target/'batch/result.json').write_text(json.dumps({stage:'pass' if stage in ('open','edit','save') else 'blocked' for stage in corpus.STAGES}))
                return 0
            with patch.object(corpus, 'ROOT', target), patch.object(corpus, 'freeze_tools', return_value=(args.binary,args.compiler,{'worker_sha256':'w','compiler_sha256':'c'})), patch.object(corpus, 'call', side_effect=worker):
                corpus.run(args)
            result = json.loads(args.report.read_text())['projects'][0]
            self.assertIn('media_record_checksum_mismatch', result['capture_classes'])
            self.assertEqual(source.read_bytes(), b'changed source bytes')

    def test_snapshot_preference_stays_within_the_selected_key_family(self):
        records = [{'key':'media:x','source':'host'}, {'key':'media:x','snapshot':'db'}, {'key':'clip','snapshot':'other'}]
        self.assertEqual(corpus.select_records('clip', {'storageKey':'media:x'}, records, Path('.')), [records[1]])

    def test_generated_rotations_match_ffmpeg_autorotate_exactly(self):
        from rotation import generate, UnsupportedFFmpeg
        with tempfile.TemporaryDirectory() as directory:
            for turns in (1,2,3):
                try:
                    witness = generate(Path(directory), turns)
                except UnsupportedFFmpeg as error:
                    self.skipTest(str(error))
                self.assertEqual(witness['turns'],turns)

    def test_projects_only_never_reads_profile_stores(self):
        import json
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            profile, target = base/'source', base/'snapshot'
            profile.mkdir()
            args = SimpleNamespace(profile=profile, projects_only=True, projects=[], extra_files=None,
                resume_snapshot=False, resume_results=False, report=base/'report.json',
                retain_for_debug=False, batch_gib=1.5, only_ids=None, binary=base/'worker', compiler=base/'helper', timeout=5)
            def worker(binary, arguments, log, **kwargs):
                self.assertEqual(arguments[0], 'inventory')
                self.assertTrue((target/'profile/store').is_dir())
                (target/'inventory.json').write_text(json.dumps({'projects':[], 'associated_files':[]}))
                return 0
            with patch.object(corpus,'ROOT',target), patch.object(corpus,'tree',side_effect=AssertionError('source profile read')), patch.object(corpus,'freeze_tools',return_value=(args.binary,args.compiler,{'worker_sha256':'w','compiler_sha256':'c'})), patch.object(corpus,'call',side_effect=worker):
                corpus.run(args)
            report = json.loads(args.report.read_text())
            self.assertEqual(report['snapshot']['host_records'],0)
            self.assertEqual(report['snapshot']['indexeddb_records'],0)
            self.assertEqual(report['library_slots'],0)
            self.assertFalse(report['gate_pass'])

    def test_dataless_project_is_blocked_before_parser_or_project_worker(self):
        import json
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            source, target = base/'source', base/'snapshot'
            source.mkdir()
            placeholder = source/'synthetic.pmv'
            args = SimpleNamespace(profile=source, projects_only=True, projects=[source], extra_files=None,
                resume_snapshot=False, resume_results=False, report=base/'report.json',
                retain_for_debug=False, batch_gib=1.5, only_ids=None, binary=base/'worker', compiler=base/'helper', timeout=5)
            def worker(binary, arguments, log, **kwargs):
                self.assertEqual(arguments[0], 'inventory')
                (target/'inventory.json').write_text(json.dumps({'projects':[], 'associated_files':[]}))
                return 0
            def external(arguments, **kwargs):
                if arguments[0] == 'rg':
                    return SimpleNamespace(returncode=0, stdout=os.fsencode(placeholder)+b'\n')
                self.assertEqual(arguments[-2:], ['--stat-one', str(placeholder)])
                return SimpleNamespace(returncode=1, stdout='', stderr='source_not_local')
            with patch.object(corpus, 'ROOT', target), patch.object(corpus, 'freeze_tools', return_value=(args.binary,args.compiler,{'worker_sha256':'w','compiler_sha256':'c'})), patch.object(corpus, 'call', side_effect=worker) as call, patch.object(corpus.subprocess, 'run', side_effect=external):
                corpus.run(args)
            result = json.loads(args.report.read_text())
            self.assertEqual(call.call_count, 1)
            self.assertEqual(result['totals']['open'], {'pass':0, 'fail':0, 'blocked':1})
            self.assertEqual(result['projects'][0]['failures'][0]['class'], 'source_not_local')
            self.assertTrue(all(state == 'blocked' for state in result['projects'][0]['stages'].values()))

    def test_preflight_uses_frozen_pair_and_fails_before_source_capture(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            profile = base/'source'
            profile.mkdir()
            args = SimpleNamespace(profile=profile, resume_snapshot=False,resume_results=False,
                report=base/'report.json',retain_for_debug=False,batch_gib=1.5,binary=base/'original',compiler=base/'original-helper',
                timeout=5, synthetic_preflight=True)
            frozen, compiler = base/'frozen-worker',base/'frozen-helper'
            with patch.object(corpus,'ROOT',base/'snapshot'), patch.object(corpus,'freeze_tools',return_value=(frozen,compiler,{})), patch.object(corpus,'call',return_value=1) as call, patch.object(corpus,'tree',side_effect=AssertionError('capture before preflight')):
                with self.assertRaisesRegex(RuntimeError,'shader_preflight_failed'):
                    corpus.run(args)
            self.assertEqual(call.call_args.args[0],frozen)
            self.assertEqual(call.call_args.kwargs['compiler'],compiler)

    def test_missing_compiler_is_a_setup_error_before_any_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory)
            worker=base/'worker'
            worker.write_bytes(b'synthetic executable')
            root=base/'frozen'
            with self.assertRaises(corpus.ShaderCompilerUnavailable):
                corpus.freeze_tools(worker,None,root)
            self.assertFalse(root.exists())

    def test_clone_is_independent_and_rejects_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, copy = root / 'source', root / 'copy'
            source.write_bytes(b'synthetic owner bytes')
            corpus.clone(source, copy)
            copy.write_bytes(b'edited private copy')
            self.assertEqual(source.read_bytes(), b'synthetic owner bytes')
            linked = root / 'link'
            linked.symlink_to(source)
            with self.assertRaisesRegex(RuntimeError, 'not_regular'):
                corpus.clone(linked, root / 'refused')
            self.assertFalse((root / 'refused').exists())

    def test_cross_volume_stream_preserves_source_and_checks_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            source,copy=root/'source',root/'copy'
            source.write_bytes(b'synthetic cross-volume payload')
            class NoClone:
                def clonefile(self,*args):
                    import ctypes,errno
                    ctypes.set_errno(errno.EXDEV)
                    return -1
            with patch.object(corpus.ctypes,'CDLL',return_value=NoClone()):
                corpus.clone(source,copy)
            self.assertEqual(copy.read_bytes(),source.read_bytes())
            copy.write_bytes(b'private edit')
            self.assertEqual(source.read_bytes(),b'synthetic cross-volume payload')

    def test_dataless_source_is_refused_before_payload_io(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            source=root/'placeholder'
            source.write_bytes(b'synthetic original')
            original=source.stat()
            metadata=SimpleNamespace(st_size=original.st_size,st_mode=original.st_mode,st_flags=0x40000000)
            with patch.object(corpus,'space'),patch.object(Path,'stat',return_value=metadata),patch.object(corpus.ctypes,'CDLL',side_effect=AssertionError('payload copy attempted')):
                with self.assertRaisesRegex(RuntimeError,'source_not_local'):
                    corpus.clone(source,root/'copy')
            self.assertFalse((root/'copy').exists())
            self.assertEqual(source.read_bytes(),b'synthetic original')

    def test_inventory_refuses_dataless_metadata_before_hashing_or_copying(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'placeholder').write_bytes(b'synthetic metadata')
            with patch.object(corpus, 'source_stat', side_effect=RuntimeError('source_not_local')), patch.object(corpus, 'digest', side_effect=AssertionError('placeholder read attempted')):
                with self.assertRaisesRegex(RuntimeError, 'source_not_local'):
                    corpus.inventory(root)

    def test_source_and_temporary_root_cannot_overlap(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as directory:
            source=Path(directory)
            original=source/'original'
            original.write_bytes(b'synthetic source')
            with patch.object(corpus,'ROOT',source/'nested-copy'):
                with self.assertRaisesRegex(RuntimeError,'source_destination_overlap'):
                    corpus.run(SimpleNamespace(profile=source))
            self.assertEqual(original.read_bytes(),b'synthetic source')
            self.assertFalse((source/'nested-copy').exists())

    def test_targeted_success_cannot_close_the_whole_corpus_gate(self):
        summary={'complete':True,'discovered':1,'full_discovered':2,'setup_failures':[],'projects':[{'stages':dict.fromkeys(corpus.STAGES,'pass')}]}
        self.assertFalse(corpus.gate_pass(summary))
        summary['full_discovered']=1
        self.assertTrue(corpus.gate_pass(summary))
        summary['projects'][0]['capture_classes']=['source_not_local']
        self.assertFalse(corpus.gate_pass(summary))
        summary['projects'][0].pop('capture_classes')
        summary['mixed_provenance']=True
        self.assertFalse(corpus.gate_pass(summary))
        summary['mixed_provenance']=False
        summary['setup_failures']=[{'class':'discovery_timeout'}]
        self.assertFalse(corpus.gate_pass(summary))

    def test_pause_kills_headless_worker_without_new_work(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            marker=root/'pause'
            binary=root/'worker'
            binary.write_text('#!/usr/bin/env python3\nfrom pathlib import Path\nimport time\nPath('+repr(str(marker))+').write_text("pause")\ntime.sleep(10)\n')
            binary.chmod(0o700)
            with patch.object(corpus,'PAUSE',marker):
                with self.assertRaises(corpus.LocalPause):
                    corpus.call(binary,[],root/'private.log',timeout=3)

    def test_pause_preserves_owned_temporary_root(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory)
            source=base/'source'
            source.mkdir()
            marker=base/'pause'
            marker.write_text('pause')
            target=base/'private-copy'
            args=SimpleNamespace(profile=source,resume_snapshot=False,resume_results=False,report=base/'report.json',retain_for_debug=False,batch_gib=1.5)
            with patch.object(corpus,'ROOT',target),patch.object(corpus,'PAUSE',marker):
                with self.assertRaises(corpus.LocalPause):
                    corpus.run(args)
            self.assertTrue(target.exists())
            self.assertFalse((target/'profile').exists())

    def test_disk_drop_kills_worker_during_batch(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            binary=root/'worker'
            binary.write_text('#!/usr/bin/env python3\nimport time\ntime.sleep(10)\n')
            binary.chmod(0o700)
            with patch.object(corpus,'space',side_effect=[None,corpus.DiskFloor('disk_floor')]):
                with self.assertRaises(corpus.DiskFloor):
                    corpus.call(binary,[],root/'private.log',timeout=3)

    def test_copy_reserve_is_a_resource_stop(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            source=root/'source'
            source.write_bytes(b'synthetic bytes')
            with patch.object(corpus,'free_bytes',return_value=corpus.FLOOR+1):
                with self.assertRaises(corpus.DiskFloor):
                    corpus.clone(source,root/'copy')
            self.assertFalse((root/'copy').exists())

    def test_error_classification_never_returns_private_text(self):
        private = 'PRIVATE_SYNTHETIC_NAME /private/media/synthetic.mov'
        self.assertEqual(corpus.classify(private), 'unclassified')
        self.assertEqual(corpus.classify('offline ' + private), 'offline_media')
        self.assertEqual(corpus.classify('shader needs the GLSL fallback runner'),'glsl_fallback_capability')
        self.assertNotIn(private, corpus.classify('export ' + private))

    def test_recheck_replaces_prior_diagnostics(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            batch,archive=root/'batch',root/'archive'
            batch.mkdir()
            (batch/'render.private.log').write_text('synthetic first attempt')
            corpus.archive_logs(batch,archive)
            (archive/'stale.private.log').write_text('earlier tool version')
            (batch/'render.private.log').write_text('synthetic recheck')
            corpus.archive_logs(batch,archive)
            self.assertEqual((archive/'render.private.log').read_text(),'synthetic recheck')
            self.assertFalse((archive/'stale.private.log').exists())

    def test_disk_stop_keeps_completed_stage_evidence(self):
        import json
        with tempfile.TemporaryDirectory() as directory:
            batch=Path(directory)
            (batch/'result.json').write_text(json.dumps({'open':'pass','render':'fail','audio':'pass','running':'edit'}))
            (batch/'render.private.log').write_text('synthetic glow needs the GLSL fallback runner')
            result=corpus.interrupted_result({'id':'0000000000000001','failures':[]},batch,123,[])
            self.assertEqual(result['stages']['open'],'pass')
            self.assertEqual(result['stages']['audio'],'pass')
            self.assertEqual(result['stages']['render'],'fail')
            self.assertEqual(result['stages']['edit'],'blocked')
            self.assertEqual([f['class'] for f in result['failures']],['glsl_fallback_capability','disk_floor'])

    def test_worker_timeout_is_bounded_and_recorded(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / 'worker'
            binary.write_text('#!/usr/bin/env python3\nimport time\ntime.sleep(10)\n')
            binary.chmod(0o700)
            self.assertEqual(corpus.call(binary, [], root / 'private.log', timeout=0.1), 124)

if __name__ == '__main__':
    unittest.main()
