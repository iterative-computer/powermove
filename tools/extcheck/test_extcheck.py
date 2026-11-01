import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import run


def trace(name="services.get", result=1):
    rows=[{"type":"header","format":"powermove.behavior","version":1},
          {"seq":1,"type":"call","call":1,"parent":None,"name":name,"args":[]},
          {"seq":2,"type":"return","call":1,"value":result},
          {"seq":3,"type":"end","valid":True,"calls":1}]
    return "\n".join(json.dumps(row) for row in rows)+"\n"


class HarnessTests(unittest.TestCase):
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


if __name__=='__main__':unittest.main()
