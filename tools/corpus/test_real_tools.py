"""Explicit real compiler replay: PM_CORPUS_CHECKS and POWERMOVE_GLSL_COMPILER required."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('corpus', Path(__file__).with_name('run.py'))
corpus = importlib.util.module_from_spec(spec)
spec.loader.exec_module(corpus)

class RealTools(unittest.TestCase):
    def test_relocated_pair_translates_all_builtin_shaders_after_originals_removed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            worker, helper = root/'worker', root/'helper'
            shutil.copy2(os.environ['PM_CORPUS_CHECKS'], worker)
            shutil.copy2(os.environ['POWERMOVE_GLSL_COMPILER'], helper)
            frozen_root = root/'frozen'
            frozen_root.mkdir()
            frozen, compiler, identities = corpus.freeze_tools(worker, helper, frozen_root)
            worker.unlink()
            helper.unlink()
            report = root/'shaders.private.json'
            definitions = Path(__file__).with_name('fixtures')/'builtin-shaders.json'
            self.assertEqual(corpus.call(frozen,['shaders',compiler,definitions,report],root/'compiler.private.log',timeout=120,compiler=compiler),0)
            rows = json.loads(report.read_text())
            self.assertEqual(len(rows),20)
            self.assertTrue(all(row['status']=='pass' for row in rows),rows)
            self.assertEqual(corpus.digest(compiler),identities['compiler_sha256'])

if __name__ == '__main__':
    unittest.main()
