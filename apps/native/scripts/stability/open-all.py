#!/usr/bin/env python3
"""Open every project in the real app and record how it went (stability item 1).

For each project: copy it to a scratch folder, launch the app in the background
(`open -g -n`, never frontmost) with that copy, a fresh or migrated profile and
`PM_NATIVE_MAIN_TRACE=1`, and read the app's `PM_MAIN` lines:
  opened       the kernel answered (ms after the open began)
  settled      every panel drew the project
  first_frame  the viewer presented a frame of the project
  failed       the open was refused (its message is what the alert shows)
  stall_ms     the main thread missed a 5 ms timer by >= 50 ms
A run that exits early is a crash (a new DiagnosticReports file is named), one
that never settles or fails within --timeout is a hang. The app is quit
normally after each run; nothing is left running.

Originals are never opened: only copies under --scratch. Usage:
  open-all.py --binary $CARGO_TARGET_DIR/release/powermove-native \
      [--profile-template DIR] [--warm] [--timeout 90] [--json out.json] PROJECT...
A folder argument adds every *.pmv inside it (the hostile corpus, a profile's
projects). Results print as one JSON line per run.
"""
import argparse, json, os, plistlib, re, shutil, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
LANE_ID = 'com.powermove.native.dev.{lane}'
REPORTS = Path.home() / 'Library/Logs/DiagnosticReports'


def front():
    return subprocess.run(['lsappinfo', 'front'], capture_output=True, text=True).stdout.strip()


def front_bundle():
    out = subprocess.run(['lsappinfo', 'info', '-only', 'bundleid', front()], capture_output=True, text=True).stdout
    m = re.search(r'(?:bundleID|CFBundleIdentifier)"?="([^"]+)"', out)
    return m.group(1) if m else ''



def bundle(binary, root, lane):
    root.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(HERE.parent / 'bundle-dev.sh'), str(binary), str(root)], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    app = root / 'Powermove.app'
    plist = app / 'Contents/Info.plist'
    with plist.open('rb') as f:
        info = plistlib.load(f)
    info['CFBundleIdentifier'] = LANE_ID.format(lane=lane)
    with plist.open('wb') as f:
        plistlib.dump(info, f)
    return app


def pids(app):
    exe = str(app / 'Contents/MacOS/powermove-native')
    out = subprocess.run(['pgrep', '-f', exe], capture_output=True, text=True).stdout
    return [int(p) for p in out.split()]


def quit_app(app, lane, pid, wait=20):
    subprocess.run(['osascript', '-e', f'tell application id "{LANE_ID.format(lane=lane)}" to quit'],
                   capture_output=True, timeout=30)
    end = time.monotonic() + wait
    while time.monotonic() < end:
        if not pids(app):
            return 'quit'
        time.sleep(.25)
    for p in pids(app):
        os.kill(p, 15)
    time.sleep(3)
    for p in pids(app):
        os.kill(p, 9)
    return 'killed'


def rss_mb(pid):
    out = subprocess.run(['ps', '-o', 'rss=', '-p', str(pid)], capture_output=True, text=True).stdout.strip()
    return round(int(out) / 1024) if out else None


def run_one(app, lane, project, profile, log, timeout, env):
    reports = set(REPORTS.glob('powermove-native*'))
    if log.exists():
        log.unlink()
    args = ['open', '-g', '-n', '-a', str(app), '--stdout', str(log), '--stderr', str(log)]
    for k, v in env.items():
        args += ['--env', f'{k}={v}']
    args += ['--args', '--profile', str(profile)]
    if project:
        args.append(str(project))
    started = time.monotonic()
    subprocess.run(args, check=True)
    result = {'project': project.name if project else None, 'events': {}, 'stalls': []}
    pid = None
    seen = 0
    while time.monotonic() - started < timeout:
        time.sleep(.1)
        ps = pids(app)
        if ps:
            pid = ps[0]
        if front_bundle() == LANE_ID.format(lane=lane):
            result['frontmost'] = True
        text = log.read_text(errors='replace') if log.exists() else ''
        for line in text[seen:].splitlines():
            if not line.startswith('PM_MAIN '):
                continue
            try:
                v = json.loads(line[8:])
            except ValueError:
                continue
            if 'open' in v:
                result['events'].setdefault(v['open'], v)
            if 'startup' in v:
                result.setdefault('startup', {})[v['startup']] = v['t']
            if 'stall_ms' in v:
                result['stalls'].append(round(v['stall_ms']))
        seen = text.rfind('\n') + 1
        ev = result['events']
        if pid and not ps:
            result['verdict'] = 'crashed'
            break
        if 'failed' in ev or ('first_frame' in ev and 'settled' in ev) or (project is None and 'shell' in ev):
            result['verdict'] = 'refused' if 'failed' in ev else 'opened'
            break
    else:
        result['verdict'] = 'hung' if pid else 'never-started'
    if pid and pids(app):
        time.sleep(1.5)  # stalls just after the first frame count too
        text = log.read_text(errors='replace')
        result['stalls'] = [round(json.loads(l[8:])['stall_ms']) for l in text.splitlines()
                            if l.startswith('PM_MAIN ') and '"stall_ms"' in l]
        result['rss_mb'] = rss_mb(pid)
        result['quit'] = quit_app(app, lane, pid)
    new = sorted(set(REPORTS.glob('powermove-native*')) - reports)
    if new:
        result['crash_reports'] = [str(p) for p in new]
        if result.get('verdict') == 'opened':
            result['verdict'] = 'crashed-on-quit'
    panics = [l for l in log.read_text(errors='replace').splitlines() if 'panicked' in l] if log.exists() else []
    if panics:
        result['panics'] = panics[:5]
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--lane', default='w21-stability')
    p.add_argument('--scratch', type=Path, default=Path('/private/tmp/pm-w21/open-all'))
    p.add_argument('--profile-template', type=Path, help='copy this profile per run (a migrated profile)')
    p.add_argument('--warm', action='store_true', help='open each project twice in one profile')
    p.add_argument('--timeout', type=float, default=90)
    p.add_argument('--json', type=Path)
    p.add_argument('projects', nargs='+', type=Path)
    a = p.parse_args()
    if shutil.disk_usage('/private/tmp').free < 25 * 1024 ** 3:
        sys.exit('STOP: less than 25 GB free')
    a.scratch.mkdir(parents=True, exist_ok=True)
    app = bundle(a.binary, Path.home() / 'Developer/pm-durable/gui' / a.lane, a.lane)
    if pids(app):
        sys.exit('the lane app is already running; quit it first')
    projects = []
    for path in a.projects:
        projects += sorted(path.glob('*.pmv')) if path.is_dir() else [path]
    env = {'PM_NATIVE_MAIN_TRACE': '1', 'PM_NATIVE_DRAW_OCCLUDED': '1', 'RUST_BACKTRACE': '1'}
    results = []
    for i, original in enumerate(projects):
        work = a.scratch / f'{i:03d}'
        shutil.rmtree(work, ignore_errors=True)
        work.mkdir(parents=True)
        copy = work / original.name
        if original.is_file() and os.access(original, os.R_OK):
            subprocess.run(['cp', '-c', str(original), str(copy)], check=False, capture_output=True)
            if not copy.exists():
                shutil.copy2(original, copy)
            os.chmod(copy, original.stat().st_mode & 0o777)
        else:
            copy = original  # a hostile shape (fifo, folder, unreadable): already scratch
        profile = work / 'profile'
        if a.profile_template:
            subprocess.run(['cp', '-cR', str(a.profile_template), str(profile)], check=True)
            (profile / 'store/library.lock').unlink(missing_ok=True)
        for run in (['cold', 'warm'] if a.warm else ['cold']):
            r = run_one(app, a.lane, copy, profile, work / f'{run}.log', a.timeout, env)
            r.update(run=run, index=i)
            print(json.dumps(r), flush=True)
            results.append(r)
            if r.get('frontmost'):
                sys.exit('STOP: the lane app became frontmost')
    if a.json:
        a.json.write_text(json.dumps(results, indent=1))
    bad = [r for r in results if r.get('verdict') not in ('opened', 'refused')]
    print(f'{len(results)} runs, {len(bad)} bad', file=sys.stderr)
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
