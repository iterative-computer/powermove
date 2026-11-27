import unittest
from stability import differing_paths, inspect_repeats
import json
from pathlib import Path
import tempfile
from PIL import Image
from test_extcheck import trace

class StabilityTests(unittest.TestCase):
    def test_paths_preserve_types_key_order_and_array_order(self):
        self.assertEqual(differing_paths({'x':True},{'x':1}),['/x'])
        self.assertEqual(differing_paths({'a':1,'b':2},{'b':2,'a':1}),['/$keys'])
        self.assertEqual(differing_paths([1,2],[2,1]),['/0','/1'])
        self.assertEqual(differing_paths({'a/b':0},{'a/b':1}),['/a~1b'])
        self.assertEqual(differing_paths({'x':None},{'x':None}),[])

    def captures(self, root, values, colors):
        outputs=[]
        for index,(value,color) in enumerate(zip(values,colors)):
            folder=root/str(index);folder.mkdir();outputs.append(folder)
            (folder/'trace.jsonl').write_text(trace())
            (folder/'interactions.json').write_text(json.dumps([{'op':'assert','result':value}]))
            Image.new('RGBA',(2,2),color).save(folder/'active.png')
        return outputs

    def test_three_repeats_inspect_interactions_even_when_trace_and_pixels_match(self):
        with tempfile.TemporaryDirectory() as temporary:
            captures=self.captures(Path(temporary),[True,True,1],[(0,0,0,255)]*3)
            result=inspect_repeats(captures,['active'])
            self.assertFalse(result['stable'])
            self.assertEqual(result['interaction_paths'],['/0/result'])
            self.assertEqual(result['trace_paths'],[])
            self.assertEqual(result['pixel_differences'],[])

    def test_visible_pixels_use_the_same_premultiplied_rule_as_the_gate(self):
        with tempfile.TemporaryDirectory() as temporary:
            captures=self.captures(Path(temporary),[None]*3,[(255,0,0,0),(0,255,0,0),(0,0,255,0)])
            self.assertTrue(inspect_repeats(captures,['active'])['stable'])
            Image.new('RGBA',(2,2),(0,0,255,255)).save(captures[-1]/'active.png')
            self.assertFalse(inspect_repeats(captures,['active'])['stable'])

    def test_two_samples_never_establish_stability(self):
        with self.assertRaises(ValueError):inspect_repeats([Path('a'),Path('b')],['active'])
