#!/usr/bin/env python3
"""Build/native tests by default; --display for visible A/B; --ime for OS input-source probe.
No credentials, screenshots or local logs are written inside the checkout.
"""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time
from p120_metrics import metrics
from gui_guard import GuiLock, check_user, wait_quiet, quiet_state

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--display', action='store_true')
parser.add_argument('--ime', action='store_true')
parser.add_argument('--no-build', action='store_true')
parser.add_argument('--p120', action='store_true',
                    help='Compare legacy CV sync/async presents against compositor-paced P-120')
parser.add_argument('--runs', type=int, default=1)
parser.add_argument('--workload', action='append', help='Run only the named workload (repeatable)')
parser.add_argument('--binary', type=Path, help='Use an already built binary for a paired control')
parser.add_argument('--seconds', type=float, default=65.)
args = parser.parse_args()
if not 1 <= args.runs <= 30 or not 10 <= args.seconds <= 70:
    parser.error('runs must be 1..30; seconds must be 10..70 (bounded GUI batches)')
native = Path(__file__).resolve().parents[3]
target = Path.home() / 'Developer/pm-target'
shots = Path(os.environ.get('PTEXT_ARTIFACTS', '/tmp/pm-p120'))
shots.mkdir(parents=True, exist_ok=True)
gui_lock = Path('/tmp/pm-shots/gui.lock')
gui_lock.parent.mkdir(exist_ok=True)
binary = args.binary or target / 'debug/pm-native-s2-spike'
os.environ['CARGO_TARGET_DIR'] = str(target)
os.chdir(native)

def disk_ok():
    s = os.statvfs('/')
    if s.f_bavail * s.f_frsize < 8 * 1024**3:
        raise RuntimeError('STOP: less than 8 GiB free')

def guarded(command):
    disk_ok()
    if command[0] == 'cargo':
        command = [str(Path.home() / '.local/bin/flock'), '/tmp/pm-build.lock', 'nice', '-n', '10', *command]
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
# PTEXT_META separates checkouts that share /tmp (each owns its symlinks).
meta = Path(os.environ.get('PTEXT_META', '/tmp/ptext-120-build'))
meta.mkdir(parents=True, exist_ok=True)
# Mirror crate depth so repository-relative specification tests still resolve.
repo_meta = meta / 'repo'
meta = repo_meta / 'apps/native'
meta.mkdir(parents=True, exist_ok=True)
for link, source in [(repo_meta / 'docs', native.parents[1] / 'docs'),
                     (meta / 'js', native / 'js')]:
    if not link.exists():
        link.symlink_to(source)
(meta / 'Cargo.toml').write_text('''[workspace]
resolver="3"
members=["spikes/s2","crates/shell","crates/design"]
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
for name, source in [('spikes/s2', native / 'spikes/s2'), ('crates/shell', native / 'crates/shell'), ('crates/design', native / 'crates/design')]:
    dest = meta / name
    dest.mkdir(parents=True, exist_ok=True)
    manifest = (source / 'Cargo.toml').read_text()
    manifest = re.sub(r'path\s*=\s*"(../../(?:vendor|crates)/[^"]+)"',
                      lambda m: 'path="' + str(meta / 'crates' / Path(m[1]).name if Path(m[1]).name in {'shell','design'} else (source / m[1]).resolve()) + '"', manifest)
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
    check_user()
    state = json.loads(subprocess.check_output([helper], text=True))
    console_check = Path('/tmp/pm-shots/is_locked.sh')
    if console_check.exists():
        status = subprocess.check_output([console_check], text=True).strip()
        if status not in {'locked', 'unlocked'}:
            raise RuntimeError('invalid console lock status: ' + status)
        state['locked'] = status == 'locked'
    if state['locked']:
        raise RuntimeError('UNVERIFIED: session locked; unlock and rerun explicitly')
    return state

if args.ime:
    ime = target / 'ptext-input-source'
    guarded(['swiftc', native / 'spikes/s2/scripts/ptext-input-source.swift', '-o', ime])
    with GuiLock():
        with open('/tmp/ptext-ime.log', 'w') as log:
            app = subprocess.Popen([binary], env={**os.environ, 'S2_TEXT': '1'}, stdout=log, stderr=log)
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


results = []
for run in range(args.runs):
    # At most 6 * 70 seconds under the GUI lock (< 10 minutes); release after every batch.
    with GuiLock():
        # sync = pre-P-120 (every frame transactional); async = no transaction unless
        # geometry moved (first cut, drawable-pool starvation); p120 = compositor pacing.
        workloads = ([('tiles-sync', {'S2_PACING':'1','S2_SYNC_PRESENT':'1','PM_GPUI_CV_DISPLAY_LINK':'1'}),
                      ('tiles-async', {'S2_PACING':'1','PM_GPUI_ASYNC_PRESENT':'1','PM_GPUI_CV_DISPLAY_LINK':'1'}),
                      ('tiles-p120', {'S2_PACING':'1'}),
                      ('timeline-sync', {'S2_TIMELINE':'1','S2_SYNC_PRESENT':'1','PM_GPUI_CV_DISPLAY_LINK':'1'}),
                      ('timeline-async', {'S2_TIMELINE':'1','PM_GPUI_ASYNC_PRESENT':'1','PM_GPUI_CV_DISPLAY_LINK':'1'}),
                      ('timeline-p120', {'S2_TIMELINE':'1'})] if args.p120 else
                     [('tiles-before', {'S2_PACING':'1','S2_BASELINE':'1'}),
                      ('tiles-after', {'S2_PACING':'1'}),
                      ('timeline-before', {'S2_TIMELINE':'1','S2_BASELINE':'1'}),
                      ('timeline-after', {'S2_TIMELINE':'1'})])
        if args.workload:
            workloads = [(label, env) for label, env in workloads if label in args.workload]
            if not workloads:
                raise RuntimeError('no matching workload')
        for label, env in workloads:
            wait_quiet()
            screen()
            processes = subprocess.check_output(['ps', '-A', '-o', 'comm='], text=True)
            if any(Path(s.strip()).name == 'pm-native-s2-spike' for s in processes.splitlines()):
                raise RuntimeError('another S2 instance is running; no timing taken')
            print(subprocess.check_output(['uptime'], text=True).strip(), flush=True)
            load_start = os.getloadavg()
            print(json.dumps({'workload': label, 'load_start': load_start}), flush=True)
            path = shots / ('ptext-' + label + '-' + str(run) + '.log')
            with path.open('w') as log:
                app = subprocess.Popen([binary], env={**os.environ,**env,'ZED_MEASUREMENTS':'1','PM_FRAME_PACING':'1'}, stdout=log, stderr=log)
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
            row = {'date_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
                   'run':run,'workload':label,'load_start':load_start,'load_end':os.getloadavg(),**metrics(path)}
            quiet_end = quiet_state()
            row['quiet_end'] = quiet_end['quiet']
            if not quiet_end['quiet']:
                row['pass'] = False
                row['presentation_evidence_valid'] = False
                row['unverified_reason'] = 'machine load or concurrent build; discard timing'
            print(json.dumps(row),flush=True)
            results.append(row)
            (shots / ('p120-results.json' if args.p120 else 'ptext-120-results.json')).write_text(
                json.dumps({'display':state,'runs':results},indent=2))
(shots / ('ptext-120-results.json' if not args.p120 else 'p120-results.json')).write_text(json.dumps({'display':state,'runs':results},indent=2))
after = [r for r in results if r['workload'].endswith('after') or r['workload'].endswith('p120')]
if not after:
    print('Diagnostic control workloads only; no gate verdict.')
    raise SystemExit(0)
if not all(r['pass'] for r in after):
    raise SystemExit(f'FAIL/UNVERIFIED: see {shots}; no 120 Hz pass claimed')
print('Measured sample PASS. Formal matrix requires --runs 30 and the paired reference conditions.' if args.runs<30 else 'PASS all 30 measured A/B runs; paired corpus/reference certification remains separate.')
