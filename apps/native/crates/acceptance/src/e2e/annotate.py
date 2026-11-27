#!/usr/bin/env python3
"""Link every frozen declaration in an owned CSV; shared triage stays read-only."""
import csv
import io
import json
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[6]
module = Path(__file__).resolve().parent
frozen = json.loads((module / 'frozen.json').read_text())
links = json.loads((module / 'traceability.json').read_text())
with (root / 'docs/native/e2e-triage.csv').open(newline='') as source:
    specs = {row['spec_file']: row for row in csv.DictReader(source)}
output = io.StringIO(newline='')
writer = csv.writer(output, lineterminator='\n')
writer.writerow(['source_id', 'spec_file', 'line', 'declaration_sha256', 'native_checks', 'workflows', 'title'])
for case in frozen['cases']:
    spec = specs[case['file']]
    checks = sorted(link['check'] for link in links if case['id'] in link['sources'])
    writer.writerow([case['id'], case['file'], case['line'], case['sha256'], ';'.join(checks),
                     spec['workflow_ids'], case['title']])
path = module.parent.parent / 'data/e2e-declarations.csv'
result = output.getvalue()
if '--write' in sys.argv:
    path.write_text(result)
elif not path.exists() or path.read_text() != result:
    raise SystemExit('Owned declaration map drift; review then run annotate.py --write')
print(f'{len(frozen["cases"])} frozen declarations mapped; shared triage read-only')
