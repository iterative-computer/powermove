#!/usr/bin/env python3
"""Annotate existing tag CSV rows only; preserve acceptance row/ID contracts.
Default verifies; --write updates only coverage_limit on the 289 tag spec rows.
Uses stdlib; run from any directory. Does not mark a planned check as a pass.
"""
import csv
import io
import json
from pathlib import Path
import sys

module = Path(__file__).resolve().parent
root = module.parents[5]
path = root / 'docs/native/e2e-triage.csv'
frozen = json.loads((module / 'frozen.json').read_text())
links = json.loads((module / 'traceability.json').read_text())
marker = ' E2E2 declaration ledger: '
by_file = {}
for case in frozen['cases']:
    by_file.setdefault(case['file'], []).append(case)
lines = path.read_text().splitlines(keepends=True)
header = next(csv.reader([lines[0]]))
column = header.index('coverage_limit')
output = [lines[0]]
seen = set()
for original in lines[1:]:
    row = next(csv.reader([original]))
    if row[0] not in by_file:
        output.append(original)
        continue
    seen.add(row[0])
    cases = by_file[row[0]]
    declarations = []
    for case in cases:
        checks = [link['check'] for link in links if case['id'] in link['sources']]
        suffix = '+'.join(checks) if checks else 'no-executable-backing'
        declarations.append(f"L{case['line']}={suffix}")
    row[column] = row[column].split(marker)[0] + marker + '; '.join(declarations) + (
        '. Exact frozen titles/loop parameters and workflow-specific missing capabilities: '
        'apps/native/crates/acceptance/src/e2e/{frozen,backings,traceability}.json. '
        'Checks are supporting evidence only; see pm-e2e-triage for executed outcomes, including the set_property Glide failure.'
    )
    buffer = io.StringIO(newline='')
    csv.writer(buffer, lineterminator='\n').writerow(row)
    output.append(buffer.getvalue())
if seen != set(by_file):
    raise SystemExit(f'Missing CSV tag specs: {sorted(set(by_file)-seen)}')
result = ''.join(output)
if '--write' in sys.argv:
    path.write_text(result)
elif path.read_text() != result:
    raise SystemExit('CSV annotation drift; review then run annotate.py --write')
print(f'{len(seen)} existing CSV rows / {len(frozen["cases"])} frozen declarations accounted for')
