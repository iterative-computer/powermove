#!/usr/bin/env python3
"""Build/native tests by default; --display for visible A/B; --ime for OS input-source probe.
No credentials, screenshots or local logs are written inside the checkout.
"""
import argparse
import json
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--display', action='store_true')
parser.add_argument('--ime', action='store_true')
parser.add_argument('--no-build', action='store_true')
parser.add_argument('--runs', type=int, default=1)
parser.add_argument('--seconds', type=float, default=65.)
args = parser.parse_args()
if not 1 <= args.runs <= 30 or not 10 <= args.seconds <= 70:
    parser.error('runs must be 1..30; seconds must be 10..70 (bounded GUI batches)')
native = Path(__file__).resolve().parents[3]
target = Path.home() / 'Developer/pm-target'
shots = Path('/tmp/pm-shots')
shots.mkdir(exist_ok=True)
os.environ['CARGO_TARGET_DIR'] = str(target)
os.chdir(native)

def disk_ok():
    s = os.statvfs('/')
    if s.f_bavail * s.f_frsize < 6 * 1024**3:
        raise RuntimeError('STOP: less than 6 GiB free')

def guarded(command):
    disk_ok()
    p = subprocess.Popen([str(x) for x in command], start_new_session=True)
    try:
        while p.poll() is None:
            disk_ok()
            try:
                p.wait(timeout=1)
            except subprocess.TimeoutExpired:
                pass
        if p.returncode:
            raise RuntimeError('command failed: ' + str(p.returncode))
    finally:
        if p.poll() is None:
            os.killpg(p.pid, signal.SIGTERM)
            p.wait()

# Temporary metadata isolates shared lanes; the source symlinks are exact checkout files.
meta = Path('/tmp/ptext-120-build')
meta.mkdir(exist_ok=True)
(meta / 'Cargo.toml').write_text('''[workspace]
resolver="3"
members=["s2","shell","design"]
[workspace.package]
version="0.1.0"
edition="2024"
license="AGPL-3.0-or-later"
[profile.dev]
debug=0
incremental=false
[profile.test]
debug=0
incremental=false
''')
for f in ['Cargo.lock', 'rust-toolchain.toml']:
    (meta / f).write_bytes((native / f).read_bytes())
for name, source in [('s2', native / 'spikes/s2'), ('shell', native / 'crates/shell'), ('design', native / 'crates/design')]:
    dest = meta / name
    dest.mkdir(exist_ok=True)
    manifest = (source / 'Cargo.toml').read_text()
    manifest = re.sub(r'path\s*=\s*"(../../(?:vendor|crates)/[^"]+)"',
                      lambda m: 'path="' + str(meta / Path(m[1]).name if Path(m[1]).name in {'shell','design'} else (source / m[1]).resolve()) + '"', manifest)
    (dest / 'Cargo.toml').write_text(manifest)
    for f in ['src', 'tests', 'data']:
        if (source / f).exists():
            link = dest / f
            if link.is_symlink() and link.resolve() != source / f:
                raise RuntimeError('temporary source symlink belongs to another checkout')
            if not link.exists():
                link.symlink_to(source / f)

if not args.no_build:
    guarded(['cargo', 'build', '--manifest-path', meta / 'Cargo.toml', '-p', 'pm-native-s2-spike', '-p', 'pm-shell', '-j', '3'])
    if not args.display and not args.ime:
        guarded([target / 'debug/pm-native-s2-spike', '--verify'])
        guarded(['cargo', 'test', '--manifest-path', meta / 'Cargo.toml', '-p', 'pm-native-s2-spike', '-p', 'pm-shell', '-p', 'pm-design', '-j', '3'])

if not args.display and not args.ime:
    probe = meta / 'probe'
    probe.mkdir(exist_ok=True)
    (probe / 'Cargo.toml').write_text('''[package]
name="ptext-bridge-probe"
version="0.1.0"
edition="2024"
[workspace]
[[bin]]
name="ptext-bridge-probe"
path="''' + str(native / 'spikes/s2/scripts/ax-bridge-probe.rs') + '''"
[dependencies]
accesskit="=0.24.1"
accesskit_macos="=0.26.3"
objc2="0.6"
objc2-app-kit="0.3"
objc2-foundation="0.3"
dispatch2="0.3.1"
[profile.dev]
debug=0
incremental=false
''')
    guarded(['cargo', 'run', '--offline', '--manifest-path', probe / 'Cargo.toml'])
    raise SystemExit(0)

# This helper reads lock/display state; it never unlocks or changes the display mode.
helper = target / 'ptext-screen-state'
guarded(['swiftc', native / 'spikes/s2/scripts/ptext-screen-state.swift', '-o', helper])
def screen():
    state = json.loads(subprocess.check_output([helper], text=True))
    if state['locked']:
        raise RuntimeError('UNVERIFIED: session locked; unlock and rerun explicitly')
    return state

class GuiLock:
    def __enter__(self):
        screen()
        self.path = shots / 'gui.lock'
        while True:
            try:
                with self.path.open('x') as f:
                    f.write('ptext\n')
                break
            except FileExistsError:
                print('GUI lock held; retry in 30 seconds', flush=True)
                time.sleep(30)
                screen()
        try:
            screen()
            # Avoid simultaneous timing apps; the lock cannot protect manually opened instances.
            processes = subprocess.check_output(['ps', '-A', '-o', 'comm='], text=True)
            if any(Path(s.strip()).name == 'pm-native-s2-spike' for s in processes.splitlines()):
                raise RuntimeError('another S2 instance is running; close it before measuring')
        except BaseException:
            self.path.unlink()
            raise
        return self
    def __exit__(self, *_):
        self.path.unlink()

if args.ime:
    ime = target / 'ptext-input-source'
    guarded(['swiftc', native / 'spikes/s2/scripts/ptext-input-source.swift', '-o', ime])
    with GuiLock():
        with open('/tmp/ptext-ime.log', 'w') as log:
            app = subprocess.Popen([target / 'debug/pm-native-s2-spike'], env={**os.environ, 'S2_TEXT': '1'}, stdout=log, stderr=log)
            try:
                time.sleep(2)
                screen()
                subprocess.run([ime], check=True)
                print('Text demo stays open for 45 seconds for the documented 30-second manual check.', flush=True)
                time.sleep(45)
                screen()
                subprocess.run(['screencapture', '-x', str(shots / 'ptext-ime.png')], check=True)
            finally:
                app.terminate()
                app.wait()
    raise SystemExit(0)

state = screen()
if state['maximum_hz'] != 120:
    raise RuntimeError('UNVERIFIED: this gate requires a 120 Hz display; detected ' + str(state))

def percentile(v, q):
    return sorted(v)[min(len(v)-1, math.ceil(q*len(v))-1)]
def metrics(path):
    content = path.read_text()
    cpu = [float(n)*{'ns': .000001, 'µs': .001, 'ms': 1, 's': 1000}[u]
           for n,u in re.findall(r'frame duration: ([\d.]+)(ns|µs|ms|s)', content)][30:]
    timestamps = [float(n) for n in re.findall(r'PM_GPUI_PRESENT ([\d.]+)', content)][30:]
    times = sorted(t for t in timestamps if t > 0)
    gaps = [b-a for a,b in zip(times,times[1:])]
    # Zero timestamps are unknown, never counted as successful presentations.
    dropped = sum(max(0, round(d*120)-1) for d in gaps)
    valid = (len(cpu) >= 6000 and len(times) >= len(cpu) and 0 not in timestamps
             and len(times) > 1 and times[-1]-times[0] >= 60 and all(d>0 for d in gaps))
    return {'cpu_samples':len(cpu), 'cpu_p50_ms':percentile(cpu,.5) if cpu else None,
            'cpu_p95_ms':percentile(cpu,.95) if cpu else None,
            'positive_presentations':len(times),'unknown_presentations':timestamps.count(0),
            'missed_120hz_intervals':dropped,'presentation_evidence_valid':valid,
            'tile_render_calls':content.count('S2_TILE_RENDER'),
            'pass':bool(valid and dropped==0 and percentile(cpu,.95)<=1000/120/2)}

results = []
for run in range(args.runs):
    # At most 4 * 70 seconds under the GUI lock; release after every batch.
    with GuiLock():
        for label, env in [('tiles-before', {'S2_PACING':'1','S2_BASELINE':'1'}),
                           ('tiles-after', {'S2_PACING':'1'}),
                           ('timeline-before', {'S2_TIMELINE':'1','S2_BASELINE':'1'}),
                           ('timeline-after', {'S2_TIMELINE':'1'})]:
            screen()
            path = Path('/tmp/ptext-' + label + '-' + str(run) + '.log')
            with path.open('w') as log:
                app = subprocess.Popen([target / 'debug/pm-native-s2-spike'], env={**os.environ,**env,'ZED_MEASUREMENTS':'1','PM_FRAME_PACING':'1'}, stdout=log, stderr=log)
                try:
                    time.sleep(3)
                    screen()
                    subprocess.run(['screencapture','-x',str(shots / ('ptext-'+label+'-'+str(run)+'.png'))],check=True)
                    deadline = time.monotonic() + args.seconds-3
                    while time.monotonic() < deadline:
                        time.sleep(max(0, min(1, deadline-time.monotonic())))
                        screen()
                        disk_ok()
                        if app.poll() is not None:
                            raise RuntimeError('S2 exited during visible run')
                finally:
                    app.terminate()
                    app.wait()
            row = {'run':run,'workload':label,**metrics(path)}
            print(json.dumps(row),flush=True)
            results.append(row)
Path('/tmp/ptext-120-results.json').write_text(json.dumps({'display':state,'runs':results},indent=2))
after = [r for r in results if r['workload'].endswith('after')]
if not all(r['pass'] for r in after):
    raise SystemExit('FAIL/UNVERIFIED: see /tmp/ptext-120-results.json; no 120 Hz pass claimed')
print('Measured sample PASS. Formal matrix requires --runs 30 and the paired reference conditions.' if args.runs<30 else 'PASS all 30 measured A/B runs; paired corpus/reference certification remains separate.')
