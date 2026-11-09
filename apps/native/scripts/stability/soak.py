#!/usr/bin/env python3
"""Long editing session on a real project, in the real app (stability item 4).

Launches a development build in the background (`open -g -n`, never
frontmost) on a copy of the project and a scratch profile, then loops until
--minutes have passed:
  edits        set_property / add_layer / delete_layers / add_effect /
               remove_effect through the app's MCP tools
  undo storms  Undo x N then Redo x N (powermove://review/command/undo|redo)
  playback     play for a few seconds, pause; step frames; back to start
  render cache render_frames at spread times
  export       one export_video per --export-every minutes
and, once half-way, kills the app (-9) and relaunches it on the same profile:
the recovery question is answered Recover, so the session continues on the
recovered work. Every 10 s it samples RSS, threads, open files, Metal memory
(footprint) and new DiagnosticReports for powermove-native; PM_MAIN stall
lines are counted. Exit 1 on a crash, a hang (an MCP call without an answer
in 60 s), or growth past the limits printed at the end.

  soak.py --binary $CARGO_TARGET_DIR/debug/powermove-native --minutes 30 PROJECT.pmv
"""
import argparse, json, os, random, re, shutil, subprocess, sys, threading, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPORTS = Path.home() / 'Library/Logs/DiagnosticReports'
CUA = Path.home() / '.local/bin/cua-driver'


class Mcp:
    """Newline JSON-RPC to the running app through its own --powermove-mcp proxy."""

    def __init__(self, exe, profile, env):
        self.p = subprocess.Popen([str(exe), '--powermove-mcp', '--profile', str(profile)],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, env=env)
        self.n = 0
        self.call_raw('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {},
                                     'clientInfo': {'name': 'soak', 'version': '1'}})
        self.p.stdin.write(json.dumps({'jsonrpc': '2.0', 'method': 'notifications/initialized'}) + '\n')
        self.p.stdin.flush()

    def call_raw(self, method, params, timeout=60):
        self.n += 1
        ident = self.n
        self.p.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': ident, 'method': method, 'params': params}) + '\n')
        self.p.stdin.flush()
        box = {}

        def read():
            while True:
                line = self.p.stdout.readline()
                if not line:
                    box['eof'] = True
                    return
                msg = json.loads(line)
                if msg.get('id') == ident:
                    box['msg'] = msg
                    return
        t = threading.Thread(target=read, daemon=True)
        t.start()
        t.join(timeout)
        if 'msg' not in box:
            raise TimeoutError(f'{method} {params.get("name", "")}: no answer in {timeout} s'
                               + (' (proxy closed)' if box.get('eof') else ''))
        return box['msg']

    def tool(self, name, args=None, timeout=60):
        msg = self.call_raw('tools/call', {'name': name, 'arguments': args or {}}, timeout)
        result = msg.get('result', {})
        text = ''.join(c.get('text', '') for c in result.get('content', []))
        try:
            value = json.loads(text)
        except ValueError:
            value = text
        return (not result.get('isError'), value)

    def close(self):
        try:
            self.p.stdin.close()
            self.p.wait(5)
        except Exception:
            self.p.kill()


def bundle(binary, root, lane):
    subprocess.run([str(HERE.parent / 'bundle-dev.sh'), str(binary), str(root)], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    app = root / 'Powermove.app'
    subprocess.run(['/usr/libexec/PlistBuddy', '-c',
                    f'Set :CFBundleIdentifier com.powermove.native.dev.{lane}',
                    str(app / 'Contents/Info.plist')], check=True)
    return app


def pids(app):
    out = subprocess.run(['pgrep', '-f', str(app / 'Contents/MacOS/powermove-native')],
                         capture_output=True, text=True).stdout
    return [int(p) for p in out.split()]


def launch(app, profile, project, log):
    (profile / 'store/library.lock').unlink(missing_ok=True)
    args = ['open', '-g', '-n', '-a', str(app), '--stdout', str(log), '--stderr', str(log)]
    for k, v in {'PM_NATIVE_MAIN_TRACE': '1', 'PM_NATIVE_DRAW_OCCLUDED': '1', 'RUST_BACKTRACE': '1'}.items():
        args += ['--env', f'{k}={v}']
    subprocess.run(args + ['--args', '--profile', str(profile), str(project)], check=True)
    for _ in range(120):
        if pids(app):
            return pids(app)[0]
        time.sleep(.25)
    raise RuntimeError('the app did not start')


def command(app, name):
    """A menu command without the menu: the development build's review link."""
    subprocess.run(['open', '-g', '-a', str(app), f'powermove://review/command/{name}'], check=True)


def front_is(lane):
    asn = subprocess.run(['lsappinfo', 'front'], capture_output=True, text=True).stdout.strip()
    info = subprocess.run(['lsappinfo', 'info', '-only', 'bundleid', asn], capture_output=True, text=True).stdout
    return f'com.powermove.native.dev.{lane}' in info


def sample(pid):
    out = {}
    ps = subprocess.run(['ps', '-o', 'rss=', '-p', str(pid)], capture_output=True, text=True).stdout.strip()
    out['rss_mb'] = round(int(ps) / 1024) if ps else None
    th = subprocess.run(['ps', '-M', '-p', str(pid)], capture_output=True, text=True).stdout
    out['threads'] = max(0, len(th.splitlines()) - 1)
    fd = subprocess.run(['lsof', '-p', str(pid)], capture_output=True, text=True).stdout
    out['fds'] = max(0, len(fd.splitlines()) - 1)
    fp = subprocess.run(['footprint', '-p', str(pid)], capture_output=True, text=True).stdout
    m = re.search(r'phys_footprint:\s+([\d.]+)\s*([KMG])B', fp)
    if m:
        out['footprint_mb'] = round(float(m.group(1)) * {'K': 1 / 1024, 'M': 1, 'G': 1024}[m.group(2)])
    m = re.search(r'([\d.]+)\s*([KMG])B\s+[\d.]+\s*[KMG]B\s+[\d.]+\s*[KMG]B\s+\d+\s+IOAccelerator', fp)
    if m:
        out['gpu_mb'] = round(float(m.group(1)) * {'K': 1 / 1024, 'M': 1, 'G': 1024}[m.group(2)])
    return out


def window_id(pid, title=None):
    out = subprocess.run([str(CUA), 'call', 'list_windows', json.dumps({'pid': pid})],
                         capture_output=True, text=True).stdout
    try:
        wins = json.loads(out).get('windows', [])
    except ValueError:
        return None
    wins = [w for w in wins if not title or title in w.get('title', '')]
    return max(wins, key=lambda w: w['bounds']['width'] * w['bounds']['height'])['window_id'] if wins else None


def press_button(pid, label):
    """Press a sheet button by its accessibility label (never a menu item)."""
    wid = window_id(pid)
    if not wid:
        return False
    out = subprocess.run([str(CUA), 'call', 'get_window_state', json.dumps(
        {'pid': pid, 'window_id': wid, 'include_screenshot': False, 'query': label})],
        capture_output=True, text=True).stdout
    try:
        state = json.loads(out)
    except ValueError:
        return False
    m = re.search(r'\[(\d+)\] AXButton "' + re.escape(label) + '"', state.get('tree_markdown', ''))
    if not m:
        return False
    subprocess.run([str(CUA), 'call', 'run_steps', json.dumps(
        {'pid': pid, 'window_id': wid, 'steps': [
            {'tool': 'click', 'args': {'element_token': f'{state["snapshot_id"]}:{m.group(1)}'}}]})],
        capture_output=True, text=True)
    return True


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--binary', type=Path, required=True, help='a development build (review links)')
    p.add_argument('--lane', default='w21-stability')
    p.add_argument('--minutes', type=float, default=30)
    p.add_argument('--export-every', type=float, default=10)
    p.add_argument('--scratch', type=Path, default=Path('/private/tmp/pm-w21/soak'))
    p.add_argument('--profile-template', type=Path)
    p.add_argument('--no-crash', action='store_true', help='skip the kill and recovery step')
    p.add_argument('--seed', type=int, default=21)
    p.add_argument('project', type=Path)
    a = p.parse_args()
    if shutil.disk_usage('/private/tmp').free < 25 * 1024 ** 3:
        sys.exit('STOP: less than 25 GB free')
    rng = random.Random(a.seed)
    shutil.rmtree(a.scratch, ignore_errors=True)
    a.scratch.mkdir(parents=True)
    project = a.scratch / a.project.name
    subprocess.run(['cp', '-c', str(a.project), str(project)], check=True)
    profile = a.scratch / 'profile'
    if a.profile_template:
        subprocess.run(['cp', '-cR', str(a.profile_template), str(profile)], check=True)
    app = bundle(a.binary, Path.home() / 'Developer/pm-durable/gui' / a.lane, a.lane)
    if pids(app):
        sys.exit('the lane app is already running; quit it first')
    exe = app / 'Contents/MacOS/powermove-native'
    env = dict(os.environ)
    if env.get('PM_FFMPEG_DIR'):
        env['DYLD_LIBRARY_PATH'] = env['PM_FFMPEG_DIR'] + '/lib'
    reports = set(REPORTS.glob('powermove-native*'))
    log = a.scratch / 'app.log'
    pid = launch(app, profile, project, log)
    started = time.monotonic()
    samples, events, failures = [], {}, []

    def note(kind):
        events[kind] = events.get(kind, 0) + 1

    def connect():
        for _ in range(240):
            if (profile / 'mcp/connection.json').exists():
                break
            time.sleep(.5)
        mcp = Mcp(exe, profile, env)
        project_id = None
        for _ in range(120):
            ok, value = mcp.tool('list_projects')
            project_id = value.get('currentProjectId') if ok and isinstance(value, dict) else None
            if project_id:
                break
            time.sleep(1)
        ok, value = mcp.tool('open_project', {'projectId': project_id})
        if not ok:
            raise RuntimeError(f'open_project: {value}')
        ok, value = mcp.tool('start_session', {'prompt': 'stability soak', 'access': 'project'})
        if not ok:
            raise RuntimeError(f'start_session: {value}')
        return mcp

    def layers(mcp):
        ok, value = mcp.tool('get_timeline', {'limit': 200})
        if not ok or not isinstance(value, dict):
            return []
        rows = value.get('strips') or []
        return [l['id'] for l in rows if isinstance(l, dict) and l.get('id') and not l.get('locked')]

    time.sleep(8)
    mcp = connect()
    crashed_once = a.no_crash
    last_export = started
    last_sample = 0.0
    cycle = 0
    try:
        while time.monotonic() - started < a.minutes * 60:
            cycle += 1
            if front_is(a.lane):
                failures.append('the app became frontmost')
                break
            if not pids(app):
                failures.append('the app exited')
                break
            ids = layers(mcp)
            # Edits.
            for _ in range(10):
                if ids:
                    target = rng.choice(ids)
                    mcp.tool('apply_commands', {'label': 'soak', 'commands': [
                        {'type': 'set_property', 'target': target, 'path': 'opacity', 'value': rng.randint(10, 100)}]})
                    note('edit')
            ok, _ = mcp.tool('apply_commands', {'commands': [
                {'type': 'add_layer', 'id': f'soak{cycle}', 'layerType': 'solid'}]})
            if ok:
                note('add_layer')
                mcp.tool('apply_commands', {'commands': [
                    {'type': 'add_effect', 'target': f'soak{cycle}', 'effect': rng.choice(['blur', 'glow', 'grain', 'vignette'])}]})
                note('add_effect')
                mcp.tool('apply_commands', {'commands': [{'type': 'delete_layers', 'targets': [f'soak{cycle}']}]})
                note('delete_layer')
            # Undo / redo storm.
            for name in ['undo'] * 15 + ['redo'] * 15:
                command(app, name)
                note(name)
            # Playback, frame steps, back to start.
            command(app, 'play')
            time.sleep(4)
            command(app, 'play')
            for _ in range(10):
                command(app, 'nextFrame')
            command(app, 'gotoStart')
            note('playback')
            # Render cache.
            ok, _ = mcp.tool('render_frames', {'times': [rng.uniform(0, 5) for _ in range(4)], 'width': 480}, timeout=120)
            note('render_frames' if ok else 'render_frames_refused')
            # Export.
            if time.monotonic() - last_export > a.export_every * 60:
                ok, value = mcp.tool('export_video', {'name': f'soak-{cycle}', 'format': 'mp4', 'range': 'work'}, timeout=600)
                note('export' if ok else 'export_refused')
                last_export = time.monotonic()
            # Crash and recover, once, half-way.
            if not crashed_once and time.monotonic() - started > a.minutes * 30:
                crashed_once = True
                mcp.close()
                os.kill(pid, 9)
                time.sleep(2)
                reports |= set(REPORTS.glob('powermove-native*'))  # a SIGKILL leaves none, but be exact
                pid = launch(app, profile, project, a.scratch / 'app-relaunch.log')
                for _ in range(60):
                    time.sleep(1)
                    if press_button(pid, 'Recover') or press_button(pid, 'Restore'):
                        note('recovered')
                        break
                mcp = connect()
            if time.monotonic() - last_sample > 10:
                s = sample(pid)
                s['t'] = round(time.monotonic() - started)
                s['cycle'] = cycle
                samples.append(s)
                print(json.dumps(s), flush=True)
                last_sample = time.monotonic()
    except (TimeoutError, RuntimeError, BrokenPipeError, ValueError) as e:
        failures.append(f'{type(e).__name__}: {e}')
        print('FAILURE', failures[-1], flush=True)
    finally:
        try:
            mcp.close()
        except Exception:
            pass
        if pids(app):
            # The soak's edits leave the project dirty: answer its save question.
            quit = subprocess.Popen(['osascript', '-e', f'tell application id "com.powermove.native.dev.{a.lane}" to quit'],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            for _ in range(60):
                if not pids(app):
                    break
                press_button(pid, 'Don\u2019t Save')
                time.sleep(1)
            quit.kill()
            for p_ in pids(app):
                os.kill(p_, 9)
                failures.append('needed a kill to quit')
    new = sorted(set(REPORTS.glob('powermove-native*')) - reports)
    if new:
        failures.append(f'crash reports: {[str(x) for x in new]}')
    text = ''.join(f.read_text(errors='replace') for f in a.scratch.glob('app*.log'))
    stalls = [float(m) for m in re.findall(r'"stall_ms":([\d.]+)', text)]
    panics = [l for l in text.splitlines() if 'panicked' in l]
    if panics:
        failures.append(f'panics: {panics[:3]}')
    third = samples[len(samples) // 3:] if len(samples) >= 6 else samples
    growth = {k: (third[-1].get(k, 0) or 0) - (third[0].get(k, 0) or 0) for k in ('rss_mb', 'threads', 'fds', 'gpu_mb')} if third else {}
    limits = {'rss_mb': 300, 'threads': 8, 'fds': 30}
    for k, limit in limits.items():
        if growth.get(k, 0) > limit:
            failures.append(f'{k} grew {growth[k]} over the last two thirds (limit {limit})')
    summary = {'minutes': round((time.monotonic() - started) / 60, 1), 'cycles': cycle, 'events': events,
               'growth_last_two_thirds': growth, 'first': samples[0] if samples else None,
               'last': samples[-1] if samples else None, 'stalls_over_50ms': len(stalls),
               'worst_stall_ms': round(max(stalls)) if stalls else 0, 'failures': failures}
    (a.scratch / 'summary.json').write_text(json.dumps({'summary': summary, 'samples': samples}, indent=1))
    print(json.dumps(summary, indent=1))
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
