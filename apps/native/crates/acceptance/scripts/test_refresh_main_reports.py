"""Pinned-main publication rejects missing provenance and mixed reference runs."""
import importlib.util
from pathlib import Path
import unittest

spec=importlib.util.spec_from_file_location("refresh_main",Path(__file__).with_name("refresh-main-reports.py"))
module=importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def fixture():
    specs=[dict(file=f"synthetic-{i}.spec.ts",in_legacy_396=i<396,in_370_roster=i<370,in_main=i<314,legacy_acceptance_ids=["A001"] if i<396 else [],main_declaration_ids=[],replacement_status="blocked-by") for i in range(397)]
    cases=[]
    for i in range(719):
        case_id=f"MAIN.E2E.synthetic-{i}"
        specs[i%314]["main_declaration_ids"].append(case_id)
        cases.append(dict(id=case_id,workflow="W001",kind="main-e2e-declaration",status="blocked-by",blocked_by="native-open",needs_macos=True,reason="Exact source declaration"))
    return dict(format="powermove.acceptance.main",reference=module.REFERENCE,scope="full-inventory",compatibility={"reference":"07ed5fa17ee640659cb075f9cad1af2095011d60","cases":[]},specs=specs,cases=cases,
                blocker_groups=[dict(backing="native-open",case_ids=[c["id"] for c in cases],workflows=["W001"],needs_macos=True)],
                workflows=[dict(id=f"W{i:03}",status="blocked-by",passed=0,failed=0,blocked=1) for i in range(1,84)])


class MainRefreshTests(unittest.TestCase):
    def test_pinned_reference_and_union_rosters_are_rendered(self):
        tracked,derived,status=module.render(fixture())
        self.assertIn(module.REFERENCE,tracked["e2e-map.csv"])
        self.assertIn("719 blocked",status)
        self.assertEqual(len(derived["remaining-blockers.csv"].splitlines()),720)
        self.assertIn("main reference",status)

    def test_old_reference_and_filtered_inputs_are_refused(self):
        report=fixture();report["reference"]="v1.2.0"
        with self.assertRaisesRegex(ValueError,"reference"):module.render(report)
        report=fixture();report["scope"]="filtered"
        with self.assertRaisesRegex(ValueError,"full"):module.render(report)

    def test_legacy_roster_and_main_declarations_cannot_disappear(self):
        report=fixture();report["specs"][0]["in_370_roster"]=False
        with self.assertRaisesRegex(ValueError,"legacy"):module.render(report)
        report=fixture();report["cases"].pop()
        with self.assertRaisesRegex(ValueError,"main declaration"):module.render(report)

    def test_wrong_backing_or_omitted_blockers_are_refused(self):
        report=fixture();report["blocker_groups"][0]["backing"]="incorrect"
        with self.assertRaisesRegex(ValueError,"backing"):module.render(report)
        report=fixture();report["blocker_groups"]=[]
        with self.assertRaisesRegex(ValueError,"partition"):module.render(report)


if __name__ == "__main__":unittest.main()
