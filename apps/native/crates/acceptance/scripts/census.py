#!/usr/bin/env python3
"""Print strict current pass/fail/blocked counts for each primary blocking reason.

The immutable starting population is exactly 2,662 main + 3,136 legacy rows.
Each row has ONE reviewed primary reason; prerequisites beyond that reason remain
in the source requirement. This reporter does not execute or manufacture passes.
Use verify.sh to execute production probes, then supply its full JSON report.
"""
import argparse
import collections
import csv
import json
from pathlib import Path

DATA = Path(__file__).resolve().parents[1] / 'data'
REFERENCES = {'main': 'c7f46a05641b23944d3692c9d80f9677c7e1ef1d',
              'v1.2.0': '07ed5fa17ee640659cb075f9cad1af2095011d60'}
CLASSES = {'portable-probe', 'macos-screen', 'macos-platform', 'unfinished-lane',
           'retained-owner-scope'}


def inventory():
    with (DATA / 'blocked-baseline-2026-11-05.csv').open(newline='') as source:
        rows = list(csv.DictReader(source))
    policies = json.loads((DATA / 'blocker-reasons-2026-11-05.json').read_text())
    keys = [(r['reference'], r['case_id']) for r in rows]
    if len(keys) != len(set(keys)) or collections.Counter(r['reference'] for r in rows) != {'main': 2662, 'v1.2.0': 3136}:
        raise ValueError('baseline must retain every original blocked row exactly once')
    if set(policies) != {r['blocked_by'] for r in rows}:
        raise ValueError('reason registry must cover every baseline backing exactly')
    for policy in policies.values():
        if policy['reason_class'] not in CLASSES or not all(policy[k] for k in ['dependency', 'reason', 'evidence']):
            raise ValueError('invalid primary reason or missing evidence')
    return rows, policies


def assess(report, rows=None, policies=None):
    if rows is None:
        rows, policies = inventory()
    if report.get('scope') != 'full-inventory' or report.get('reference') != REFERENCES['main']:
        raise ValueError('full pinned-main report required')
    legacy = report.get('compatibility')
    if not legacy or legacy.get('reference') != REFERENCES['v1.2.0'] or legacy.get('scope') != 'full-inventory':
        raise ValueError('full separate legacy report required')
    cases = {}
    for ref, source in [('main', report), ('v1.2.0', legacy)]:
        for case in source['cases']:
            key = (ref, case['id'])
            if key in cases:
                raise ValueError('duplicate result')
            if case['status'] not in {'pass', 'fail', 'blocked-by'}:
                raise ValueError('unknown verdict')
            if (case['status'] == 'blocked-by') != bool(case.get('blocked_by')):
                raise ValueError('verdict and backing disagree')
            if not case.get('reason'):
                raise ValueError('result lacks evidence or failure reason')
            cases[key] = case
    counts = collections.defaultdict(collections.Counter)
    table = []
    for row in rows:
        key = (row['reference'], row['case_id'])
        if key not in cases:
            raise ValueError('baseline assertion disappeared: ' + str(key))
        case = cases.pop(key)
        policy = policies[row['blocked_by']]
        category = policy['reason_class']
        # A portable candidate must execute; keep failed implementation/probes
        # visibly fail. A missing worker remains blocked, never an implicit pass.
        counts[(row['reference'], category)][case['status']] += 1
        table.append(dict(reference=row['reference'], case_id=row['case_id'],
                          baseline_backing=row['blocked_by'], reason_class=category,
                          dependency=policy['dependency'], status=case['status'],
                          current_backing=case.get('blocked_by') or '',
                          reason=policy['reason'], evidence=policy['evidence']))
    for (ref, _), case in cases.items():
        if case['status'] == 'blocked-by':
            # Configuring the worker is mandatory for the requested starting
            # census. Extra unconfigured rows are visible, not silently omitted.
            category = 'additional-unconfigured-backing'
        elif case['id'] in {'MAIN.file.utf16-ingress', 'W007.mcp-utf16-ingress'}:
            category = 'additional-file-ingress-probe'
        else:
            category = 'outside-starting-blockers'
        counts[(ref, category)][case['status']] += 1
    return counts, table


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('report', type=Path)
    parser.add_argument('--csv', type=Path, help='write anonymized row-level current verdicts')
    parser.add_argument('--strict', action='store_true')
    args = parser.parse_args()
    counts, table = assess(json.loads(args.report.read_text()))
    print('reference reason pass fail blocked')
    for (ref, reason), count in sorted(counts.items()):
        print(ref, reason, count['pass'], count['fail'], count['blocked-by'])
    for ref in REFERENCES:
        total = sum((v for (r, _), v in counts.items() if r == ref), collections.Counter())
        print(ref, 'TOTAL', total['pass'], total['fail'], total['blocked-by'])
    if args.csv:
        with args.csv.open('w', newline='') as output:
            writer = csv.DictWriter(output, fieldnames=list(table[0]), lineterminator='\n')
            writer.writeheader()
            writer.writerows(table)
    if args.strict:
        if any(v['fail'] for v in counts.values()):
            raise SystemExit(1)
        if any(v['blocked-by'] for v in counts.values()):
            raise SystemExit(2)


if __name__ == '__main__':
    main()
