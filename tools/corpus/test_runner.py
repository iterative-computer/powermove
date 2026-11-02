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

class CorpusSafety(unittest.TestCase):
    def setUp(self):
        # Unit fixtures are only a few bytes. Model capacity explicitly so
        # another lane's builds cannot change a timeout/pause test's meaning.
        capacity=patch.object(corpus.shutil,'disk_usage',return_value=SimpleNamespace(free=corpus.FLOOR+1024**3))
        capacity.start()
        self.addCleanup(capacity.stop)

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
            with patch.object(corpus.shutil,'disk_usage',return_value=SimpleNamespace(free=corpus.FLOOR+1)):
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
