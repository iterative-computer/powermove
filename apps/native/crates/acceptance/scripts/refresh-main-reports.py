#!/usr/bin/env python3
"""Write pinned-main evidence: tracked failure/coverage maps in data/, derived
blocker lists next to the report; append shared W1 status."""
import collections
import csv
import io
import json
from pathlib import Path
import sys

REFERENCE = "c7f46a05641b23944d3692c9d80f9677c7e1ef1d"
MARKER = "main-acceptance-c7f46a0"


def csv_text(fields, rows):
    stream = io.StringIO(newline="")
    writer = csv.writer(stream, lineterminator="\n")
    writer.writerow(fields)
    writer.writerows(rows)
    return stream.getvalue()


def render(report):
    if report.get("format") != "powermove.acceptance.main" or report.get("reference") != REFERENCE:
        raise ValueError("expected the pinned main reference")
    if report.get("scope") != "full-inventory" or report.get("compatibility") is None:
        raise ValueError("full main and compatibility inventories required")
    if report["compatibility"].get("reference") != "07ed5fa17ee640659cb075f9cad1af2095011d60":
        raise ValueError("separate legacy compatibility reference required")
    if any(".corpus-" in c["id"] for c in report["compatibility"]["cases"]):
        raise ValueError("public embedded fixtures only")
    specs = report["specs"]
    if len(specs) != len({s["file"] for s in specs}) or sum(s["in_legacy_396"] for s in specs) != 396 or sum(s["in_370_roster"] for s in specs) != 370:
        raise ValueError("legacy rosters must be retained")
    cases = {c["id"]: c for c in report["cases"]}
    if len(cases) != len(report["cases"]):
        raise ValueError("duplicate main assertions")
    declarations = {c["id"] for c in cases.values() if c["kind"] == "main-e2e-declaration"}
    if len(declarations) != 719 or sum(s["in_main"] for s in specs) != 314:
        raise ValueError("pinned main declaration/spec inventory changed")
    if declarations != {i for s in specs for i in s["main_declaration_ids"]}:
        raise ValueError("main declaration links differ")
    grouped = [i for g in report["blocker_groups"] for i in g["case_ids"]]
    if len(grouped) != len(set(grouped)) or set(grouped) != {c["id"] for c in cases.values() if c["status"] == "blocked-by"}:
        raise ValueError("incomplete blocker partition")
    for group in report["blocker_groups"]:
        if any(cases[i]["blocked_by"] != group["backing"] for i in group["case_ids"]):
            raise ValueError("incorrect backing group")
    fields = ["case_id", "workflow", "kind", "needs_macos", "blocked_by", "reason"]
    rows = lambda status: [[c["id"], c["workflow"], c["kind"], str(c["needs_macos"]).lower(), c["blocked_by"] or "", c["reason"]] for c in cases.values() if c["status"] == status]
    blockers = csv_text(fields, rows("blocked-by"))
    failures = csv_text(fields, rows("fail"))
    mapping = csv_text(["spec_file", "in_370_roster", "in_legacy_396", "in_main", "reference", "legacy_acceptance_ids", "main_declaration_ids", "replacement_status"],
                       [[s["file"], str(s["in_370_roster"]).lower(), str(s["in_legacy_396"]).lower(), str(s["in_main"]).lower(), REFERENCE,
                         ";".join(s["legacy_acceptance_ids"]), ";".join(s["main_declaration_ids"]), s["replacement_status"]] for s in specs])
    groups = csv_text(["backing", "blocked_assertions", "needs_macos", "workflows", "case_ids"],
                      [[g["backing"], len(g["case_ids"]), str(g["needs_macos"]).lower(), ";".join(g["workflows"]), ";".join(g["case_ids"])] for g in report["blocker_groups"]])
    counts = collections.Counter(c["status"] for c in cases.values())
    status = "\n".join([f"<!-- {MARKER}:start -->", "## Native execution status — main reference, 2026-11-04", "",
                        f"Active behavior reference: `{REFERENCE}`. This supersedes earlier tagged parity counts;",
                        "v1.2.0 remains a separate required file/extension compatibility inventory.",
                        f"Current main assertions: **{counts['pass']:,} pass / {counts['fail']} fail / {counts['blocked-by']:,} blocked**.",
                        "Schema/routing/supporting passes do not certify a complete tool body, native key dispatch or workflow.",
                        "All 83 current workflow contracts remain blocked; main declarations retain 1 measured failure and 718 blockers.",
                        "The union map preserves all 396 legacy specs and the frozen 370 roster; main adds one further spec.", "",
                        "| Workflow | Status | Pass | Fail | Blocked | Remaining backing |", "|---|---|---:|---:|---:|---|",
                        *[f'| {w["id"]} | {w["status"]} | {w["passed"]} | {w["failed"]} | {w["blocked"]} | '
                          + "; ".join(sorted({c["blocked_by"] for c in cases.values() if c["workflow"] == w["id"] and c["blocked_by"]})) + " |" for w in report["workflows"]], "",
                        "A01–A10 current contracts are independently gated as `MAIN.DELTA.A01` through `MAIN.DELTA.A10`.",
                        "P rows route to delta-data, E rows to delta-fx, U rows to the later UI lane; retaining their source requirements does not claim implementation.",
                        "A511/A512's old discrepancy is superseded: main explicitly requires project memory and indexing.",
                        "The native tool/list and missing index now fail against that source; no additional catalog-change approval is inferred.",
                        "See [acceptance.md](acceptance.md#main-reference-change--2026-11-04), the current failures CSV and the archived legacy snapshots.",
                        "", f"<!-- {MARKER}:end -->", ""])
    return {"failures.csv": failures, "e2e-map.csv": mapping}, {"remaining-blockers.csv": blockers, "blocker-groups.csv": groups}, status


def main():
    root = Path(__file__).resolve().parents[5]
    report_path = Path(sys.argv[1])
    report = json.loads(report_path.read_text())
    tracked, derived, status = render(report)
    path = root / "docs/native/w1-workflows.md"
    previous = path.read_text()
    if f"<!-- {MARKER}:start -->" in previous and status.rstrip() not in previous:
        raise ValueError("append a new evidence section instead of rewriting shared history")
    for name, value in tracked.items():
        (root / "apps/native/crates/acceptance/data" / name).write_text(value)
    for name, value in derived.items():
        report_path.with_name(name).write_text(value)
    if f"<!-- {MARKER}:start -->" not in previous:
        path.write_text(previous.rstrip() + "\n\n" + status)


if __name__ == "__main__":
    main()
