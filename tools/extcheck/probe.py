#!/usr/bin/env python3
"""Three native V8 recordings per public case; component evidence, never P2 matches."""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import run

EXPRESSIONS = [
    '__extcheckClock.snapshot()',
    '(async()=>{await __extcheckClock.settle(3);return {clock:__extcheckClock.snapshot(),project:PM.proj,time:PM.time,playing:PM.playing,errors:__extcheck.errors}})()',
    '(()=>{PM.pause();PM.setTime(.5);return [PM.time,PM.playing]})()',
    '__extcheck.dispose()',
]


def record(runner, output):
    request = {
        'compat_bundle': str(output/'compat.js'), 'clock_fixture': str(output/'clock.js'),
        'module': str(output/'extension.js'), 'manifest': json.loads((output/'manifest.json').read_text()),
        'initial': json.loads((output/'initial.json').read_text()),
        'expressions': EXPRESSIONS, 'inspect_dom': True,
    }
    result = subprocess.run([str(runner)], input=json.dumps(request).encode(), capture_output=True, timeout=90)
    if result.returncode:raise run.Blocked('native-runner-failed')
    return json.loads(result.stdout)


def inspect_recordings(recordings):
    if len(recordings) < 3:raise ValueError('At least three recordings are required')
    if any(not isinstance(item,dict) or not isinstance(item.get('activation'),dict) for item in recordings):
        return 'native-probe-invalid'
    if any(item.get('activation', {}).get('ok') is not True for item in recordings):
        return 'native-activation-needs-backend'
    if any(not isinstance(item.get('results'),list) or len(item['results'])!=4
           or not isinstance(item['results'][1],dict) or not isinstance(item.get('diagnostics'),dict)
           for item in recordings):return 'native-probe-invalid'
    if any(item.get('diagnostics', {}).get('missing') or item['results'][1]['errors'] for item in recordings):
        return 'native-probe-needs-backend'
    if any(run.exact(item) != run.exact(recordings[0]) for item in recordings[1:]):
        return 'native-nondeterministic'
    return 'repeatable-component-probe'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference-root', type=Path, required=True)
    parser.add_argument('--native-runner', type=Path, required=True)
    parser.add_argument('--corpus-root', type=Path, default=run.ROOT/'apps/desktop/e2e/fixtures/compat-corpus')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.resolve().is_relative_to(run.ROOT):parser.error('Private diagnostics belong outside the public checkout')
    counts = Counter()
    run.WORK.mkdir(mode=0o700, exist_ok=True)
    cases = {}
    for index, source in enumerate(run.discover(args.corpus_root), 1):
        case = f'local-corpus-{index:04d}'
        run.disk_check()
        with tempfile.TemporaryDirectory(prefix='native-probe-', dir=run.WORK) as temporary:
            root = Path(temporary)
            try:
                sha = run.copy_extension(source, root/'input')
                output = root/'output'
                output.mkdir()
                built = subprocess.run(['bun', str(Path(__file__).with_name('bundle.ts')),
                    str(args.reference_root), str(root/'input'), str(output)], capture_output=True, timeout=90)
                if built.returncode:
                    raise run.Blocked('invalid-reference-manifest' if b'FROZEN_MANIFEST_INVALID' in built.stderr else 'native-extension-compile-failed')
                recordings = [record(args.native_runner, output) for _ in range(3)]
                for sample, recording in enumerate(recordings):
                    (output/f'probe-{sample}.json').write_text(json.dumps(recording))
                result = inspect_recordings(recordings)
                cases[case] = {'reason': result, 'recordings': len(recordings),
                               'digests': [run.digest(run.exact(item).encode()) for item in recordings]}
                if run.tree_digest(root/'input') != sha:raise run.Blocked('input-mutated')
                counts[result] += 1
            except (run.Blocked, subprocess.TimeoutExpired) as error:
                reason = str(error) if isinstance(error, run.Blocked) else 'native-probe-timeout'
                counts[reason] += 1
                cases[case] = {'reason': reason, 'recordings': 0}
        print(f'{case}: completed', flush=True)
    report = {'evidence': 'native-component-only', 'gate': 'OPEN', 'p2_population': 58,
              'accepted_matches': 0, 'clock_fixture': run.fixture_identity(), 'counts': dict(counts), 'cases': cases}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(args.output, os.O_WRONLY|os.O_CREAT|os.O_TRUNC, 0o600)
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, 'w') as output:output.write(json.dumps(report, indent=2)+'\n')
    print(json.dumps({'evidence': report['evidence'], 'counts': report['counts']}))
    return 0 if sum(counts.values()) == counts['repeatable-component-probe'] else 2


if __name__ == '__main__':raise SystemExit(main())
