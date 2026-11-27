#!/usr/bin/env python3
"""Release-only native fix diagnostics. Disposable fixtures, bounded GUI batches.
These command origins are not HID input or a paired Electron gate certification.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

import driver
from macos import host_seconds
from trace import analyze, continuity
from budgets import distribution


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--project', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--hz', type=int, choices=[60, 120], required=True)
    p.add_argument('--runs', type=int, default=30)
    p.add_argument('--seconds', type=int, default=60)
    p.add_argument('--audio', action='store_true')
    p.add_argument('--lock-owner', default='perffix', help='Name written to the GUI lock (alphanumeric)')
    p.add_argument('--interact', action='store_true', help='Leave a disposable registered app on screen for inspector input during the window')
    args = p.parse_args()
    assert 1 <= args.runs <= 60 and 1 <= args.seconds <= 120
    assert args.lock_owner.isalnum()
    assert args.project.resolve().is_relative_to(Path('/tmp').resolve())
    assert args.output.parent.resolve().is_relative_to(Path('/tmp').resolve()) and not args.output.is_symlink()
    args.native = Path.home() / 'Developer/pm-target/release/powermove-native'
    assert args.native.is_file(), 'Build release before acquiring GUI lock'
    # Display switches, launch, and all measurement share one owned lock.
    driver.LOCK.parent.mkdir(exist_ok=True)
    while True:
        result = subprocess.run(['zsh', '-c', f'set -o noclobber; echo {args.lock_owner} > /tmp/pm-shots/gui.lock'], capture_output=True)
        if result.returncode == 0:
            break
        print('GUI lock held; retry in 30 s', flush=True)
        time.sleep(30)
    inode = driver.LOCK.stat().st_ino
    def expire(*_):
        raise driver.BatchExpired('Native GUI batch limit (9 minutes)')
    old_alarm = signal.signal(signal.SIGALRM, expire)
    old_term = signal.signal(signal.SIGTERM, expire)
    signal.alarm(540)
    app = None
    original = None
    data = {'hz': args.hz, 'runs': [], 'gate_eligible': False,
            'present_mode': os.environ.get('PM_ENGINE_PRESENT', 'default')}
    try:
        conditions = driver.conditions(driver.Processes())
        data['before'] = conditions
        blockers = driver.execution_blockers(conditions, diagnostic=True)
        assert not blockers, blockers
        display = Path(__file__).with_name('display.swift')
        result = subprocess.run(['swift', str(display), str(args.hz)], capture_output=True, text=True, check=True)
        original = json.loads(result.stdout)
        data['display'] = original
        os.environ['PM_NATIVE_VERIFY_HZ'] = str(args.hz)
        with tempfile.TemporaryDirectory(prefix='pm-perffix-', dir='/tmp') as root:
            root = Path(root)
            import shutil
            project = root / 'document.json'
            shutil.copyfile(args.project, project)
            if args.interact:
                import plistlib
                bundle = root / 'Powermove Native Perf.app/Contents'
                (bundle / 'MacOS').mkdir(parents=True)
                executable = bundle / 'MacOS/powermove-native'
                shutil.copy2(args.native, executable)
                (bundle / 'Info.plist').write_bytes(plistlib.dumps(dict(
                    CFBundleExecutable='powermove-native', CFBundleIdentifier='local.powermove.perffix',
                    CFBundleName='Powermove Native Perf', CFBundlePackageType='APPL', NSHighResolutionCapable=True)))
                args.native = executable
            app = driver.App('native', args, root)
            for i in range(args.runs):
                observations = {}
                for op, kw in [('open', {'path': str(project)})] if i == 0 else []:
                    observations[op] = app.action(op, **kw)
                if not args.audio and not args.interact:
                    try:
                        driver.position(app, 0.)
                        driver.position(app, 1 / 30)
                        driver.position(app, 0.)
                        observations['scrub'] = app.action('seek', seconds=1 / 30)
                        driver.position(app, 0.)
                        observations['edit'] = app.action('edit', value=37 if i % 2 == 0 else 73)
                    except Exception as error:
                        observations['error'] = str(error)
                data['runs'].append(observations)
                if observations.get('error'):
                    break
            if not args.interact:
                app.request('play')
            data['interactive'] = args.interact
            data['play_start'] = host_seconds() + 1
            # Warm up separately; join all trace rows to the same Quartz clock.
            time.sleep(1)
            time.sleep(args.seconds)
            data['play_end'] = host_seconds()
            app.request('pause')
            if args.audio:
                data['seek'] = app.action('seek', seconds=.5)
                app.request('play')
                time.sleep(3)
                app.request('pause')
            time.sleep(.3)
            data['last_status'] = app.request('status')
            app.close()
            app = None
            rows = [json.loads(line) for line in (args.output.parent / (root.name + '-native.trace.json')).read_text().splitlines()]
            measured = [r for r in rows if r.get('display') and data['play_start'] <= r['display'] <= data['play_end']]
            data['playback'] = analyze(measured)
            stamps = sorted(r['display'] for r in measured if r.get('gpu_ok'))
            # Raw scanout-to-scanout gaps; with held frames these are expected
            # whenever content is slower than the display. Drops use continuity.
            data['playback']['presentation_gaps'] = sum((b - a) > 1.5 / args.hz for a, b in zip(stamps, stamps[1:]))
            doc = json.loads(args.project.read_text())
            fps = float(doc.get('fps') or 30)
            frame_ticks = round(driver.TICKS / fps)
            loop_end = round(float(data['last_status'].get('duration') or doc.get('dur') or 0) * driver.TICKS) or None
            data['playback']['continuity'] = continuity(measured, frame_ticks, args.hz, loop_end)
            data['playback']['held_frames'] = data['last_status'].get('held_frames')
            for key, a, b in [('render_ms','render_start','render_submitted'), ('drawable_wait_ms','drawable_asked','drawable_acquired'), ('commit_to_display_ms','commit','display')]:
                data['playback'][key] = distribution([(r[b]-r[a])*1000 for r in measured if r.get(a) and r.get(b) and r[b] >= r[a]])
            data['trace_path'] = str(args.output.parent / (root.name + '-native.trace.json'))
        data['after'] = driver.conditions(driver.Processes())
    except BaseException as error:
        data['error'] = str(error)
    finally:
        signal.alarm(0)  # leave the bounded teardown margin inside ten minutes
        if app:
            try:
                app.close()
            except BaseException as error:
                data['cleanup_error'] = str(error)
        if original:
            try:
                restored = subprocess.run(['swift', str(Path(__file__).with_name('display.swift')), 'restore', str(original['display']), str(original['original_mode'])], capture_output=True, text=True, timeout=15)
                data['display_restore_ok'] = restored.returncode == 0
            except BaseException as error:
                data['display_restore_ok'] = False
            if not data['display_restore_ok']:
                data['display_restore_error'] = 'Original display/mode unavailable or changed; measurements cannot certify pinned conditions.'

        signal.signal(signal.SIGALRM, old_alarm)
        signal.signal(signal.SIGTERM, old_term)
        if driver.LOCK.exists() and driver.LOCK.stat().st_ino == inode:
            driver.LOCK.unlink()
        args.output.write_text(json.dumps(data, indent=2))
    print(json.dumps(data, indent=2))
    return 1 if data.get('error') else 0


if __name__ == '__main__':
    raise SystemExit(main())
