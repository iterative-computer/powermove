import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("gate_e", Path(__file__).with_name("gate_e_inventory.py"))
gate_e = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate_e)


class InventoryTests(unittest.TestCase):
    def setUp(self):
        self.rows = {"A": {"state": "native", "condition": "known input", "evidence": ["proof"]}}
        self.ledger = {"reasons": self.rows, "references": {"A": ["a.rs:1"]}}

    def test_new_category_cannot_escape_the_ledger(self):
        self.assertTrue(gate_e.audit(self.ledger, ["A", "B"], self.ledger["references"]))

    def test_new_producer_requires_review_even_for_an_existing_category(self):
        refs = copy.deepcopy(self.ledger["references"])
        refs["A"].append("new.rs:3")
        self.assertTrue(gate_e.audit(self.ledger, ["A"], refs))

    def test_missing_evidence_or_invalid_disposition_fails(self):
        self.rows["A"] = {"state": "probably", "condition": "guess"}
        self.assertEqual(len(gate_e.audit(self.ledger, ["A"], self.ledger["references"])), 2)

    def test_complete_inventory_is_accepted(self):
        self.assertEqual(gate_e.audit(self.ledger, ["A"], self.ledger["references"]), [])

    def test_comment_removal_preserves_source_locations(self):
        source = "// Feature::Ghost\n/* Feature::Ghost\n */\nFeature::Actual"
        stripped = gate_e.without_comments(source)
        self.assertNotIn("Ghost", stripped)
        self.assertEqual(source.count("\n"), stripped.count("\n"))


if __name__ == "__main__":
    unittest.main()
