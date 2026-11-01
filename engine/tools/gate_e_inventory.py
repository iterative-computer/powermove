#!/usr/bin/env python3
"""Audit every capability category and production reference against Gate E.

References deliberately include consumers as a conservative superset of
producers. This static inventory does not prove an empty runtime unsupported set.
"""
import argparse
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
LEDGER = ROOT / "docs/engine/gate-e.json"


def without_comments(source):
    # Retain newlines so the emitted source locations remain useful.
    return re.sub(r'/\*.*?\*/|//[^\n]*',
                  lambda m: '\n' * m[0].count('\n'), source, flags=re.S)


def inventory(root=ROOT):
    source = (root / "engine/crates/pm-render/src/capability.rs").read_text()
    body = source.split("pub enum Feature {", 1)[1].split("\n}", 1)[0]
    variants = re.findall(r'^    (\w+)(?:\([^\n]*\))?,', without_comments(body), re.M)
    references = {}
    for path in sorted((root / "engine/crates/pm-render/src").rglob("*.rs")):
        source = path.read_text().split("#[cfg(test)]", 1)[0]
        source = without_comments(source)
        # Display formatting is a consumer, not a producer; retain it anyway
        # so an audit cannot miss a constructor through an over-tight filter.
        for match in re.finditer(r'\bFeature::(\w+)', source):
            feature = match[1]
            location = f"{path.relative_to(root)}:{source.count(chr(10), 0, match.start()) + 1}"
            references.setdefault(feature, []).append(location)
    return variants, references


def audit(ledger, variants, references):
    problems = []
    rows = ledger.get("reasons", {})
    if set(rows) != set(variants):
        problems.append(f"unclassified/missing variants: {sorted(set(rows) ^ set(variants))}")
    if set(references) - set(variants):
        problems.append(f"unknown production variants: {sorted(set(references) - set(variants))}")
    for variant in variants:
        row = rows.get(variant, {})
        if row.get("state") not in ("native", "needs-macOS", "blocked-by"):
            problems.append(f"{variant}: missing disposition")
        if not row.get("evidence") or not row.get("condition"):
            problems.append(f"{variant}: missing evidence/condition")
    if ledger.get("references") != references:
        problems.append("production reference inventory changed; review and regenerate --write")
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true")
    parser.add_argument("--require-green", action="store_true")
    args = parser.parse_args()
    ledger = json.loads(LEDGER.read_text())
    variants, references = inventory()
    if args.write:
        ledger["references"] = references
        LEDGER.write_text(json.dumps(ledger, indent=2) + "\n")
    problems = audit(ledger, variants, references)
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        return 1
    blockers = [name for name, row in ledger["reasons"].items()
                if row["state"] != "native" or row.get("verified") is not True]
    open_items = [name for name, row in ledger.get("work_items", {}).items()
                  if row.get("verified") is not True]
    print(f"{len(variants)} categories, {sum(map(len, references.values()))} production references accounted for")
    print(f"Gate E {'OPEN' if blockers or open_items else 'GREEN'}: "
          f"{len(blockers)} unqualified categories, {len(open_items)} open work items")
    return int(args.require_green and bool(blockers or open_items))


if __name__ == "__main__":
    raise SystemExit(main())
