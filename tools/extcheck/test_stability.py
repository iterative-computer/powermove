import unittest
from stability import differing_paths

class StabilityTests(unittest.TestCase):
    def test_paths_preserve_types_key_order_and_array_order(self):
        self.assertEqual(differing_paths({'x':True},{'x':1}),['/x'])
        self.assertEqual(differing_paths({'a':1,'b':2},{'b':2,'a':1}),['/$keys'])
        self.assertEqual(differing_paths([1,2],[2,1]),['/0','/1'])
        self.assertEqual(differing_paths({'a/b':0},{'a/b':1}),['/a~1b'])
        self.assertEqual(differing_paths({'x':None},{'x':None}),[])
