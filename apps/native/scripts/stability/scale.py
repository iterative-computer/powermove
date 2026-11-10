#!/usr/bin/env python3
"""A large project in the real app: open time, edit round trips, main-thread
stalls and memory (scale lane).

Generate the project first (outputs stay out of git):
  cargo run --release -p pm-kernel --features synthetic --example scale_project -- 5000 /private/tmp/scale-5000.pmv
then:
  scale.py --binary $CARGO_TARGET_DIR/soak/powermove-native /private/tmp/scale-5000.pmv

Launches the app in the background on a copy (soak.py's launcher), waits for
the project to settle, then times --edits rounds of a move, a keyframe, a
rename and a visibility toggle on a layer of the open composition through the
app's MCP tools (the round trip includes the kernel thread and publication).
Prints one JSON summary; exits 1 when an edit's median round trip is over a
60 Hz frame or the app stalls its main thread for more than --stall-ms.
"""
import argparse, json, re, shutil, statistics, subprocess, sys, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import soak  # noqa: E402

FRAME_MS = 1000 / 60


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--lane', default='w22-scale')
    p.add_argument('--scratch', type=Path, default=Path('/private/tmp/pm-w22-scale/scale'))
    p.add_argument('--edits', type=int, default=15)
    p.add_argument('--stall-ms', type=float, default=100)
    p.add_argument('--layer', default='L1000', help='a layer id of the open composition')
    p.add_argument('project', type=Path)
    a = p.parse_args()
    shutil.rmtree(a.scratch, ignore_errors=True)
    a.scratch.mkdir(parents=True)
    project = a.scratch / a.project.name
    subprocess.run(['cp', '-c', str(a.project), str(project)], check=True)
    app = soak.bundle(a.binary, Path.home() / 'Developer/pm-durable/gui' / a.lane, a.lane)
    if soak.pids(app):
        sys.exit('the lane app is already running; quit it first')
    log = a.scratch / 'app.log'
    profile = a.scratch / 'profile'
    pid = soak.launch(app, profile, project, log)
    result, failures = {}, []
    try:
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline and '"open":"settled"' not in log.read_text(errors='replace'):
            time.sleep(0.5)
        for _ in range(240):
            if (profile / 'mcp/connection.json').exists():
                break
            time.sleep(0.5)
        env = dict(**__import__('os').environ)
        mcp = soak.Mcp(app / 'Contents/MacOS/powermove-native', profile, env)
        ok, value = mcp.tool('list_projects')
        project_id = value.get('currentProjectId') if ok and isinstance(value, dict) else None
        ok, value = mcp.tool('open_project', {'projectId': project_id})
        ok, value = mcp.tool('start_session', {'prompt': 'scale check', 'access': 'project'})
        if not ok:
            raise RuntimeError(f'start_session: {value}')
        before = soak.sample(pid)
        commands = {
            'move': lambda n: {'type': 'set_property', 'target': a.layer, 'path': 'position.x', 'value': 100 + n},
            'keyframe': lambda n: {'type': 'set_property', 'target': a.layer, 'path': 'rotation', 'value': n,
                                   'mode': 'keyframe', 'time': 0.5 + n * 0.1},
            'rename': lambda n: {'type': 'set_layer', 'target': a.layer, 'patch': {'name': f'Renamed {n}'}},
            'toggle': lambda n: {'type': 'set_layer', 'target': a.layer, 'patch': {'visible': n % 2 == 1}},
        }
        rounds = {}
        for kind, make in commands.items():
            times, refused = [], None
            for n in range(a.edits):
                t = time.perf_counter()
                ok, value = mcp.tool('apply_commands', {'label': kind, 'commands': [make(n)]})
                times.append((time.perf_counter() - t) * 1000)
                if not ok and refused is None:
                    refused = str(value)[:200]
            rounds[kind] = {'median_ms': round(statistics.median(times), 2), 'worst_ms': round(max(times), 2)}
            if refused:
                rounds[kind]['refused'] = refused
                failures.append(f'{kind} refused: {refused}')
            elif statistics.median(times) > FRAME_MS:
                failures.append(f'{kind}: median round trip {statistics.median(times):.1f} ms is over a frame')
        result['edits'] = rounds
        result['before_edits'], result['after_edits'] = before, soak.sample(pid)
        mcp.close()
    except (TimeoutError, RuntimeError, ValueError) as e:
        failures.append(f'{type(e).__name__}: {e}')
    finally:
        subprocess.Popen(['osascript', '-e', f'tell application id "com.powermove.native.dev.{a.lane}" to quit'],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(40):
            if not soak.pids(app):
                break
            soak.press_button(pid, 'Don’t Save')
            time.sleep(1)
        for p_ in soak.pids(app):
            __import__('os').kill(p_, 9)
    text = log.read_text(errors='replace')
    events = {}
    for line in text.splitlines():
        if line.startswith('PM_MAIN ') and '"open"' in line:
            v = json.loads(line[8:])
            events.setdefault(v['open'], v.get('ms'))
    stalls = [float(m) for m in re.findall(r'"stall_ms":([\d.]+)', text)]
    result['open'] = events
    result['stalls_over_50ms'] = len(stalls)
    result['worst_stall_ms'] = round(max(stalls)) if stalls else 0
    if stalls and max(stalls) > a.stall_ms:
        failures.append(f'main thread stalled {max(stalls):.0f} ms')
    result['failures'] = failures
    print(json.dumps(result, indent=1))
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
