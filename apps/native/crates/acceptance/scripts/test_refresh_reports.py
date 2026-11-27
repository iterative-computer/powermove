"""Portable report/ownership guards; Python 3.9+ standard library only."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("refresh_reports", Path(__file__).with_name("refresh-reports.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def fixture():
    specs = [dict(file=f"synthetic-{i}.spec.ts", in_370_roster=i < 370, classification="mixed",
                  acceptance_ids=["A001"], supporting_probe_ids=[], replacement_status="blocked-by", reason="Synthetic")
             for i in range(396)]
    cases = [dict(id="A001", workflow="W001", kind="original", status="blocked-by", needs_macos=True, blocked_by="native-open", reason="Exact unverified assertion"),
             dict(id="W001.failed", workflow="W001", kind="supporting-probe", status="fail", needs_macos=False, blocked_by=None, reason="Measured authored-curve defect")]
    report = dict(scope="full-inventory", specs=specs, cases=cases,
                  workflows=[dict(id=f"W{i:03}", status="blocked-by", passed=0, failed=0, blocked=1) for i in range(1,84)],
                  blocker_groups=[dict(backing="native-open", case_ids=["A001"], needs_macos=True, workflows=["W001"])])
    triage = [dict(spec_file=s["file"], provenance="retain exactly") for s in specs]
    return report, triage


class RefreshTests(unittest.TestCase):
    def test_render_keeps_failures_blockers_and_rosters_distinct(self):
        report, triage = fixture()
        before = copy.deepcopy(triage)
        blockers, mapping, groups, status = module.render(report, triage)
        self.assertIn("Exact unverified assertion", blockers)
        self.assertNotIn("Measured authored-curve defect", blockers)
        self.assertIn("0 pass / 1 fail / 1 blocked", status)
        self.assertEqual(len(mapping.splitlines()),397)
        self.assertIn("native-open,1,true,W001,A001", groups)
        self.assertEqual(triage,before)

    def test_filtered_and_external_corpus_reports_are_refused(self):
        report,triage = fixture()
        report["scope"] = "filtered"
        with self.assertRaisesRegex(ValueError,"complete"):
            module.render(report,triage)
        report["scope"] = "full-inventory"
        report["cases"][0]["id"] = "W007.corpus-0000"
        with self.assertRaisesRegex(ValueError,"synthetic"):
            module.render(report,triage)

    def test_missing_duplicate_and_wrong_group_members_fail_closed(self):
        for case_ids in [[],["A001","A001"],["W001.failed"],["unknown"]]:
            report,triage=fixture()
            report["blocker_groups"][0]["case_ids"] = case_ids
            with self.assertRaisesRegex(ValueError,"partition"):
                module.render(report,triage)

        report,triage=fixture()
        report["blocker_groups"][0]["backing"] = "unrelated-adapter"
        with self.assertRaisesRegex(ValueError,"metadata"):
            module.render(report,triage)

    def test_frozen_roster_and_shared_provenance_are_not_rewritten(self):
        report,triage=fixture()
        report["specs"][0]["in_370_roster"]=False
        with self.assertRaisesRegex(ValueError,"370"):
            module.render(report,triage)
        report,triage=fixture()
        triage[0]["spec_file"]="unknown.spec.ts"
        with self.assertRaisesRegex(ValueError,"shared triage"):
            module.render(report,triage)


if __name__ == "__main__":
    unittest.main()
