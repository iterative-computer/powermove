#!/usr/bin/env python3
"""Write the legacy coverage map to data/ and derived blocker lists next to the
report; append W1 evidence and never write shared triage."""
import collections
import csv
import io
import json
from pathlib import Path
import sys

STAMP = "2026-11-04"


def quote(value):
    return '"' + str(value).replace('"', '""') + '"'


def render(report, triage):
    if report.get("scope") != "full-inventory" or len(report["workflows"]) != 83:
        raise ValueError("refresh requires a complete acceptance inventory")
    if any(".corpus-" in case["id"] for case in report["cases"]):
        raise ValueError("refresh accepts embedded public synthetic fixtures only")
    cases = {case["id"]: case for case in report["cases"]}
    specs = {spec["file"]: spec for spec in report["specs"]}
    if len(cases) != len(report["cases"]) or len(specs) != 396:
        raise ValueError("duplicate cases or changed spec inventory")
    if {row["spec_file"] for row in triage} != set(specs) or len(triage) != 396:
        raise ValueError("shared triage roster drift")
    if sum(spec["in_370_roster"] for spec in specs.values()) != 370:
        raise ValueError("frozen 370 roster drift")
    grouped = [case_id for group in report["blocker_groups"] for case_id in group["case_ids"]]
    blocked = {c["id"] for c in cases.values() if c["status"] == "blocked-by"}
    if len(grouped) != len(set(grouped)) or set(grouped) != blocked:
        raise ValueError("blocker groups must partition every blocked case exactly once")
    for group in report["blocker_groups"]:
        members = [cases[case_id] for case_id in group["case_ids"]]
        if (any(case["blocked_by"] != group["backing"] for case in members)
                or sorted(group["workflows"]) != sorted({case["workflow"] for case in members})
                or group["needs_macos"] != any(case["needs_macos"] for case in members)):
            raise ValueError("blocker group backing or capability metadata differs from its cases")
    blockers = "case_id,workflow,kind,needs_macos,blocked_by,reason\n"
    for case in cases.values():
        if case["status"] == "blocked-by":
            blockers += ",".join([quote(case["id"]), quote(case["workflow"]), quote(case["kind"]),
                                  str(case["needs_macos"]).lower(), quote(case["blocked_by"]), quote(case["reason"])]) + "\n"
    mapping = "spec_file,in_370_roster,classification,acceptance_ids,supporting_probe_ids,replacement_status,reason\n"
    for spec in report["specs"]:
        mapping += ",".join([quote(spec["file"]), str(spec["in_370_roster"]).lower(), quote(spec["classification"]),
                             quote(";".join(spec["acceptance_ids"])), quote(";".join(spec["supporting_probe_ids"])),
                             spec["replacement_status"], quote(spec["reason"])]) + "\n"
    groups = io.StringIO(newline="")
    writer = csv.writer(groups, lineterminator="\n")
    writer.writerow(["backing", "blocked_assertions", "needs_macos", "workflows", "case_ids"])
    for group in report["blocker_groups"]:
        writer.writerow([group["backing"], len(group["case_ids"]), str(group["needs_macos"]).lower(),
                         ";".join(group["workflows"]), ";".join(group["case_ids"])])
    counts = collections.Counter(c["status"] for c in cases.values())
    status = "\n".join([
        f"<!-- native-acceptance-status:{STAMP}:start -->", f"## Native execution status — {STAMP}", "",
        "This dated run supersedes the earlier Wave 5 status table without rewriting its evidence.",
        f"Configured Linux: **{counts['pass']:,} pass / {counts['fail']} fail / {counts['blocked-by']:,} blocked**.",
        "All 83 complete workflow contracts remain blocked; a supporting failure makes W061's aggregate fail.",
        "All 396 triaged specs and the frozen 370 roster are retained. Shared e2e-triage.csv is historical input;",
        "the lane-owned e2e-map.csv holds current coverage; a run's remaining-blockers.csv lists open assertions.",
        "See [acceptance.md](acceptance.md#parity-follow-up--2026-11-04).", "",
        "| Workflow | Status | Pass | Fail | Blocked | Remaining backing |", "|---|---|---:|---:|---:|---|",
        *[f'| {w["id"]} | {w["status"]} | {w["passed"]} | {w["failed"]} | {w["blocked"]} | '
          + "; ".join(sorted({c["blocked_by"] for c in cases.values() if c["workflow"] == w["id"] and c["blocked_by"]})) + " |"
          for w in report["workflows"]], "", f"<!-- native-acceptance-status:{STAMP}:end -->", "",
    ])
    return blockers, mapping, groups.getvalue(), status


def main():
    root = Path(__file__).resolve().parents[5]
    report_path = Path(sys.argv[1])
    report = json.loads(report_path.read_text())
    with (root / "docs/native/e2e-triage.csv").open(newline="") as source:
        triage = list(csv.DictReader(source))
    blockers, mapping, groups, status = render(report, triage)
    path = root / "docs/native/w1-workflows.md"
    previous = path.read_text()
    marker = f"<!-- native-acceptance-status:{STAMP}:start -->"
    # Never replace a shared status table. Identical replay is a no-op; a changed
    # replay needs a new dated section, preserving the earlier evidence.
    if marker in previous and status.rstrip() not in previous:
        raise ValueError("dated W1 evidence differs; append a new dated section")
    (root / "apps/native/crates/acceptance/data/legacy-v1.2.0-e2e-map.csv").write_text(mapping)
    for name, content in [("remaining-blockers.csv", blockers), ("blocker-groups.csv", groups)]:
        report_path.with_name("legacy-v1.2.0-" + name).write_text(content)
    if marker not in previous:
        path.write_text(previous.rstrip() + "\n\n" + status)


if __name__ == "__main__":
    main()
