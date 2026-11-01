#!/usr/bin/env python3
"""Viewer-2 on-screen check and playback soak for the real powermove-native app.

Runs, in one GUI-locked batch (< 10 minutes):
  1. output switch SDR -> PQ -> HLG -> SDR (each must present a new generation),
  2. engine-drawn safe areas + guides,
  3. popout viewer (second window, same device) presenting,
  4. display moves of the editor and the popout when more than one screen exists
     (display id / EDR headroom before and after, frame presented after),
  5. a playback soak (default 60 s) with the popout open, sampling viewer status
     and summarising the PM_ENGINE_TRACE presentation stream per viewer.

Uses the opt-in PM_PERF_GATE stdin seam and a disposable /tmp profile. Never
touches a real profile, never unlocks the Mac, never removes another lane's
GUI lock. Window-only screenshots go to --shots (default /tmp/pm-viewer2).

Usage (after `source engine/env.sh` and building pm-native-app):
  python3 apps/native/crates/app/tools/viewer2-soak.py [--project /tmp/x.json]
      [--seconds 60] [--hz 120] [--binary PATH] [--no-lock-check]
"""
import argparse, json, math, os, signal, subprocess, sys, tempfile, threading, time
from pathlib import Path

p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
p.add_argument('--project', type=Path, help='disposable /tmp project; default: generated shapes project')
p.add_argument('--seconds', type=int, default=60)
p.add_argument('--hz', type=int, choices=[60, 120], default=120)
p.add_argument('--binary', type=Path, default=Path.home() / 'Developer/pm-target/debug/powermove-native')
p.add_argument('--shots', type=Path, default=Path('/tmp/pm-viewer2'))
p.add_argument('--owner', default='viewer2', help='GUI lock owner name')
p.add_argument('--soak-popout', action='store_true', help='keep the popout open during the playback soak')
p.add_argument('--no-lock-check', action='store_true', help='diagnostic run while locked: no screenshots, no verdicts')
a = p.parse_args()
assert 1 <= a.seconds <= 240, 'soak must stay inside the 10-minute batch'
a.shots.mkdir(parents=True, exist_ok=True)
here = Path(__file__).resolve().parent
st = os.statvfs('/')
assert st.f_bavail * st.f_frsize >= 6 * 1024 ** 3, 'STOP: less than 6 GiB free'


def locked():
    out = subprocess.run(['ioreg', '-n', 'Root', '-d1', '-a'], capture_output=True, text=True).stdout
    i = out.find('CGSSessionScreenIsLocked')
    return i >= 0 and '<true/>' in out[i:i + 80]


onscreen = not a.no_lock_check
if onscreen and locked():
    sys.exit('Mac session is locked; unlock it (or pass --no-lock-check for a diagnostic run).')
winid = a.shots / 'winid'
if onscreen and not winid.exists():
    subprocess.run(['swiftc', '-O', '-o', str(winid), str(here / 'winid.swift')], check=True)

root = Path(tempfile.mkdtemp(prefix='pm-native-viewer2.', dir='/tmp'))
if a.project:
    project = a.project.resolve()
    assert project.is_relative_to(Path('/tmp').resolve()), 'use a disposable /tmp project copy'
else:
    # The unchanged public shape-primitives corpus fixture (animated, 640x360).
    project = root / 'project.json'
    project.write_text((here.parents[1] / 'engine-service/tests/fixtures/shapes.json').read_text())

LOCK = Path('/tmp/pm-shots/gui.lock')
lock_owned = False
if onscreen:
    LOCK.parent.mkdir(exist_ok=True)
    deadline = time.monotonic() + 600
    while True:
        try:
            with LOCK.open('x') as f:
                f.write(a.owner + '\n')
            lock_owned = True
            break
        except FileExistsError:
            print('Waiting for GUI owner:', LOCK.read_text().strip(), flush=True)
            if time.monotonic() > deadline:
                sys.exit('GUI lock not acquired in 600 s; never remove another lane\'s lock')
            time.sleep(30)


def expired(*_):
    raise SystemExit('Batch exceeded 9 minutes; tearing down the owned app')


signal.signal(signal.SIGALRM, expired)
signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
signal.alarm(540)
trace = root / 'viewer.trace.json'
env = dict(os.environ, TMPDIR='/tmp', PM_PERF_GATE='1', PM_NATIVE_VERIFY_HZ=str(a.hz), PM_ENGINE_TRACE=str(trace))
if env.get('PM_FFMPEG_DIR'):
    env['DYLD_LIBRARY_PATH'] = env['PM_FFMPEG_DIR'] + '/lib'
app = None
results = {'root': str(root), 'onscreen': onscreen, 'hz': a.hz, 'soak_popout': a.soak_popout, 'load_before': os.getloadavg()[0], 'steps': []}
lock = threading.Lock()


def perf(op, **kw):
    with lock:
        app.stdin.write(json.dumps(dict(op=op, **kw)) + '\n'); app.stdin.flush()
        while True:
            line = app.stdout.readline()
            if not line:
                raise RuntimeError('app exited; see ' + str(root / 'stderr.log'))
            if line.startswith('PM_PERF '):
                r = json.loads(line[8:])
                if 'error' in r and r['error'] and op != 'status' and isinstance(r['error'], str) and 'accepted_host_s' not in r:
                    raise RuntimeError(f'{op}: {r["error"]}')
                return r


def wait(pred, timeout=20, what='condition'):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        s = perf('status')
        if pred(s):
            return s
        time.sleep(.1)
    raise TimeoutError(f'{what} not reached in {timeout} s: {json.dumps(s)[:600]}')


def presented(s, after):
    f = s.get('frame') or {}
    return (f.get('display_host_s') or 0) > after and f.get('gpu_ok')


def windows():
    out = subprocess.run([str(winid), str(app.pid)], capture_output=True, text=True).stdout
    return [l.split(' ', 6) for l in out.splitlines() if l.strip()]


def shot(name, popout=False):
    if not onscreen:
        return None
    ws = [w for w in windows() if w[5] == '0']
    ws = [w for w in ws if (' — Viewer' in w[6]) == popout] or ws
    if not ws:
        return None
    w = max(ws, key=lambda w: float(w[3]) * float(w[4]))
    path = a.shots / f'{name}.png'
    subprocess.run(['screencapture', '-x', '-o', '-l', w[0], str(path)], check=True)
    return str(path)


def step(name, **data):
    data['name'] = name
    results['steps'].append(data)
    print('STEP', json.dumps(data), flush=True)


try:
    with open(root / 'stderr.log', 'w') as errors:
        app = subprocess.Popen([str(a.binary), '--profile', str(root / 'profile'), str(project)], env=env,
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, text=True,
                               start_new_session=True)
        assert 'ready' in app.stdout.readline()
        t0 = time.monotonic()

        def front():
            if onscreen:
                subprocess.run(['osascript', '-e', 'tell application "System Events" to set frontmost of '
                                f'(first process whose unix id is {app.pid}) to true'], capture_output=True)
        time.sleep(1)
        wait(lambda s: s['opened'] and not s['opening'], 90, 'project open')
        # The frame rendered before the window is visible has no presentedTime;
        # re-seek until a paused frame is actually shown.
        end = time.monotonic() + 30
        while True:
            front()
            perf('seek', seconds=0.5 + (time.monotonic() % 1))
            time.sleep(.5)
            s = perf('status')
            if presented(s, 0):
                break
            if time.monotonic() > end:
                raise TimeoutError('no presented frame in 30 s: ' + json.dumps(s)[:400])
        step('open', seconds=round(time.monotonic() - t0, 2), main_output=s['main_output'], message=s['message'])
        front()
        time.sleep(.5)
        perf('seek', seconds=1.0)
        for mode in ['sdr', 'pq', 'hlg', 'sdr']:
            before = perf('status')
            g0 = before.get('generation') or 0
            t = time.monotonic()
            perf('output', mode=mode)
            s = wait(lambda s: s['output_mode'] == mode and (s.get('generation') or 0) > g0
                     and presented(s, before['accepted_host_s']), 10, f'{mode} frame')
            step(f'output-{mode}', ms=round((time.monotonic() - t) * 1000), generation=s['generation'],
                 message=s['message'], shot=shot(f'output-{mode}') if mode != 'sdr' or g0 else None)
        before = perf('status')
        perf('overlays', safe=True, guides=True)
        s = wait(lambda s: s['safe_areas'] and s['guides'] and presented(s, before['accepted_host_s']), 10, 'overlay frame')
        step('overlays', shot=shot('overlays-safe-guides'))
        before = perf('status')
        perf('popout', open=True)
        s = wait(lambda s: s['popout'] and ((s['popout'].get('frames') or {}).get('display_host_s') or 0) > 0, 20,
                 'popout presented frame')
        time.sleep(.5)
        step('popout', output=s['popout']['output'], frames=s['popout']['frames'], error=s['popout']['error'],
             shot=shot('popout', popout=True), editor_shot=shot('popout-editor'))
        screens = perf('screens')['screens']
        step('screens', screens=screens)
        if len(screens) > 1:
            for window in ['main', 'popout']:
                key = 'main_output' if window == 'main' else None
                out = lambda s: s['main_output'] if window == 'main' else (s['popout'] or {}).get('output')
                start = out(perf('status'))
                order = sorted(range(len(screens)), key=lambda i: screens[i]['display'] == (start or {}).get('display'))
                for target in order[:1] + [order[-1]]:
                    before = perf('status')
                    perf('move', screen=target, window=window)
                    want = screens[target]['display']
                    s = wait(lambda s: (out(s) or {}).get('display') == want, 10, f'{window} on display {want}')
                    if window == 'main':
                        s = wait(lambda s: presented(s, before['accepted_host_s']), 10, 'frame after move')
                    else:
                        s = wait(lambda s: ((s['popout'].get('frames') or {}).get('display_host_s') or 0)
                                 > before['accepted_host_s'], 10, 'popout frame after move')
                    time.sleep(.3)
                    step(f'move-{window}', from_output=start, to_output=out(s), screen=target,
                         target_headroom=screens[target]['headroom'],
                         shot=shot(f'move-{window}-screen{target}', popout=window == 'popout'))
                    start = out(s)
        else:
            step('move', skipped='one screen; no physical display move possible')
        # Soak: overlays on, SDR output; popout open only with --soak-popout.
        if not a.soak_popout:
            perf('popout', open=False)
        front()
        time.sleep(.5)
        start = perf('status')
        perf('play')
        t_start = time.monotonic()
        samples = []
        while time.monotonic() - t_start < a.seconds:
            time.sleep(.5)
            s = perf('status')
            samples.append(s)
        perf('pause')
        end = perf('status')
        results['window'] = {'start_host': start['accepted_host_s'] + 1, 'end_host': end['accepted_host_s']}
        step('soak', seconds=a.seconds,
             errors=sorted({str(s.get('error')) for s in samples if s.get('error')}),
             popout_errors=sorted({str((s.get('popout') or {}).get('error')) for s in samples if (s.get('popout') or {}).get('error')}),
             skipped_targets=end['skipped_targets'] - start['skipped_targets'],
             no_drawable=end['no_drawable'] - start['no_drawable'],
             held_frames=end['held_frames'] - start['held_frames'],
             stale_rejections=end['stale_rejections'] - start['stale_rejections'],
             popout_no_drawable=(end['popout'] or {}).get('no_drawable'),
             metal_allocated_bytes=end['metal_allocated_bytes'])
        perf('popout', open=False)
        perf('quit')
        app.wait(timeout=30)
        app = None
finally:
    signal.alarm(0)
    if app and app.poll() is None:
        os.killpg(app.pid, signal.SIGTERM)
        try:
            app.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(app.pid, signal.SIGKILL)
    if lock_owned:
        LOCK.unlink()
    results['load_after'] = os.getloadavg()[0]

# Presentation stream per viewer inside the soak window.
rows = []
if trace.exists():
    rows = [json.loads(l) for l in trace.read_text().splitlines() if l.strip()]
w = results.get('window')
summary = {}
if w:
    interval = 1 / a.hz
    for vid in sorted({r['viewer_id'] for r in rows}):
        rs = [r for r in rows if r['viewer_id'] == vid and w['start_host'] <= r.get('worker', 0) < w['end_host']]
        shown = sorted(r['display'] for r in rs if (r.get('display') or 0) > 0)
        gaps = [y - x for x, y in zip(shown, shown[1:])]
        costs = sorted((r['completed'] - r['worker']) * 1000 for r in rs if r.get('completed', 0) > r.get('worker', 0))
        pct = lambda v, q: v[math.ceil(len(v) * q) - 1] if v else None
        # A 30 fps composition holds each picture ~4 refreshes at 120 Hz: a
        # continuity gap is longer than 1.5 content-frame intervals.
        content = max(interval, 1 / 30)
        summary[str(vid)] = {'submitted': len(rs), 'positive_presentations': len(shown),
                             'zero_or_missing': len(rs) - len(shown),
                             'gaps_over_1_5_content_frames': sum(g > 1.5 * content for g in gaps),
                             'max_gap_ms': round(max(gaps) * 1000, 3) if gaps else None,
                             'worker_to_completion_ms_p50_p95': [pct(costs, .5), pct(costs, .95)],
                             'gpu_failures': sum(1 for r in rs if not r.get('gpu_ok', True))}
results['presentation'] = summary
out = root / 'viewer2-soak.json'
out.write_text(json.dumps(results, indent=2))
print('RESULT', json.dumps({'presentation': summary, 'load': [results['load_before'], results['load_after']]}))
print('Report:', out)
