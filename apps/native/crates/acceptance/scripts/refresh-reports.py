#!/usr/bin/env python3
"""Refresh only lane-owned acceptance status from a full synthetic JSON run."""
import csv
import json
from pathlib import Path
import re
import sys

root = Path(__file__).resolve().parents[5]
report = json.loads(Path(sys.argv[1]).read_text())
if report.get("scope") != "full-inventory" or len(report["workflows"]) != 83:
    raise SystemExit("refresh requires a complete acceptance inventory")
if any(".corpus-" in case["id"] for case in report["cases"]):
    raise SystemExit("refresh accepts embedded public synthetic fixtures only")
cases = {case["id"]: case for case in report["cases"]}
specs = {spec["file"]: spec for spec in report["specs"]}
triage_path = root / "docs/native/e2e-triage.csv"
with triage_path.open(newline="") as source:
    reader = csv.DictReader(source)
    fields, rows = reader.fieldnames, list(reader)
assert len(rows) == len(specs) == 396
for row in rows:
    spec = specs[row["spec_file"]]
    targeted = [cases[case_id] for case_id in spec["acceptance_ids"]]
    row.update(
        triage_status=spec["replacement_status"],
        acceptance_ids=";".join(spec["acceptance_ids"]),
        supporting_probe_ids=";".join(spec["supporting_probe_ids"]),
        needs_macos=str(any(case["needs_macos"] for case in targeted)).lower(),
        blocked_by=";".join(sorted({case["blocked_by"] for case in targeted if case["blocked_by"]})),
    )
with triage_path.open("w", newline="") as output:
    writer = csv.DictWriter(output, fields, lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows)

data = root / "apps/native/crates/acceptance/data"
with (data / "remaining-blockers.csv").open("w", newline="") as output:
    # Match Report::blockers_csv, whose boolean column is deliberately unquoted.
    output.write("case_id,workflow,kind,needs_macos,blocked_by,reason\n")
    for case in report["cases"]:
        if case["status"] != "blocked-by":
            continue
        quote = lambda s: '"' + str(s).replace('"', '""') + '"'
        output.write(
            ",".join([quote(case["id"]), quote(case["workflow"]), quote(case["kind"]),
                      str(case["needs_macos"]).lower(), quote(case["blocked_by"]), quote(case["reason"])]) + "\n"
        )

path = root / "docs/native/w1-workflows.md"
text = path.read_text()
start, end = "<!-- native-acceptance-status:start -->", "<!-- native-acceptance-status:end -->"
status = "\n".join([
    start, "## Native execution status", "",
    "Wave 5 Linux configured run; portable supporting probes are separate from full workflow verdicts.",
    "All 396 legacy spec replacements remain blocked. See [acceptance.md](acceptance.md) for evidence and limits;",
    "[remaining-blockers.csv](../../apps/native/crates/acceptance/data/remaining-blockers.csv) names every remaining case and exact unverified assertion.", "",
    "| Workflow | Status | Pass | Fail | Blocked | Remaining backing |",
    "|---|---|---:|---:|---:|---|",
    *[f'| {w["id"]} | {w["status"]} | {w["passed"]} | {w["failed"]} | {w["blocked"]} | '
      + "; ".join(sorted({c["blocked_by"] for c in report["cases"] if c["workflow"] == w["id"] and c["blocked_by"]})) + " |"
      for w in report["workflows"]],
    "", end,
])
if start in text:
    text = re.sub(re.escape(start) + r".*?" + re.escape(end), lambda _: status, text, flags=re.S)
else:
    text = text.replace("## Workflow contracts", status + "\n\n## Workflow contracts", 1)
path.write_text(text)
