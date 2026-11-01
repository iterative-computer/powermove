#!/usr/bin/env python3
"""Sequential S3b locked-screen experiments. No input posting or activation.
Run after building: python3 apps/native/spikes/s3/bench.py [--quick] [--hidden]
Raw logs/traces stay outside Git. Hidden CAMetalLayer is NOT physical scanout.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import signal
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--quick', action='store_true')
parser.add_argument('--hidden', action='store_true')
parser.add_argument('--release', action='store_true')
parser.add_argument('--case', choices=['sync-120','async-120','async-60','scrub-uncached','scrub-cached'])
args = parser.parse_args()
if args.release:
    os.environ['S3_PROFILE'] = 'release'
root = Path(__file__).resolve().parents[4]
os.chdir(root)
seconds = 3 if args.quick else 60
cases = [('sync-120', ['--legacy-sync', '--no-cache', '--hz=120'], seconds),
         ('async-120', ['--hz=120'], seconds),
         ('async-60', ['--hz=60'], seconds),
         ('scrub-uncached', ['--synthetic-scrub', '--no-cache'], 9),
         ('scrub-cached', ['--synthetic-scrub'], 9)]
if args.hidden:
    cases = [('drawable-hidden', ['--hidden'], 3)]
if args.case:
    cases = [c for c in cases if c[0] == args.case]
for name, flags, duration in cases:
    # Build activity needs time to settle before a budget measurement. Bounded;
    # no GUI interaction or request to unlock is involved.
    deadline = time.monotonic() + 180
    while os.getloadavg()[0] >= 12:
        if time.monotonic() >= deadline:
            raise RuntimeError('No quiet measurement window (load remains >= 12)')
        time.sleep(1)
    prefix = Path('/tmp/s3b-' + ('release-' if args.release else '') + name)
    run = ['zsh', 'apps/native/spikes/s3/run.sh', '--seconds=' + str(duration),
           '--report=' + str(prefix) + '.json', *([] if args.hidden else ['--offscreen']), *flags]
    before = os.getloadavg()
    loads = [before[0]]
    with Path(str(prefix) + '.log').open('w') as log:
        process = subprocess.Popen(run, stdout=log, stderr=log, start_new_session=True)
        try:
            for _ in range(540):
                try:
                    status = process.wait(timeout=1)
                    if status:
                        raise RuntimeError(f"{name} failed ({status}); see {prefix}.log")
                    break
                except subprocess.TimeoutExpired:
                    loads.append(os.getloadavg()[0])
                    s = os.statvfs('/')
                    if s.f_bavail * s.f_frsize < 4 * 1024**3:
                        raise RuntimeError('S3 STOP: below 4 GiB free')
            else:
                raise TimeoutError('S3 batch exceeded 540 seconds')
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=10)
    result = json.loads(Path(str(prefix) + '.json').read_text())
    result['measurement_conditions'] = {'load_1m_before': before[0], 'load_1m_max': max(loads),
                                        'load_under_12': max(loads) < 12}
    Path(str(prefix) + '.json').write_text(json.dumps(result, indent=2))
    print(json.dumps({'case': name, 'load_before': before,
          'mode': result['mode'], 'frames': result['measured_frames'],
          'stages': result['stages'], 'cached_scrub': result['cached_scrub'],
          'presented_callbacks': result['presented_callbacks'], 'no_drawable': result['no_drawable']}), flush=True)
