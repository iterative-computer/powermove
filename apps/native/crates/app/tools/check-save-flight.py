#!/usr/bin/env python3
"""Save-in-flight + MCP-refresh check for powermove-native (lifecycle row).

Usage: check-save-flight.py <binary> <layers> [--close]
  layers=60000 reproduces the large-project case; layers=0 the small project.
Builds a disposable project under /tmp, opens it in the app (PM_PERF_GATE seam,
PM_NATIVE_MAIN_TRACE probe), commits an MCP add_layer through the app's own
--powermove-mcp connection, then starts an MCP save and (with --close) asks the
editor to close 0.1 s into it. Reports when the editor saw the edit, the
main-thread stalls (>= 50 ms) per phase, the status notes shown during the save
and whether the window stayed open. Takes /tmp/pm-shots/gui.lock (noclobber,
waits, never removes a foreign lock); window-only shots go to $SHOTS
(default /tmp/pm-viewer2). Refuses to run while the Mac is locked.
"""

import json, os, pathlib, subprocess, sys, tempfile, threading, time

binary, layers = sys.argv[1], int(sys.argv[2])
_ioreg = subprocess.run(['ioreg', '-n', 'Root', '-d1', '-a'], capture_output=True, text=True).stdout
_i = _ioreg.find('CGSSessionScreenIsLocked')
if _i >= 0 and '<true/>' in _ioreg[_i:_i + 80]:
    sys.exit('Mac session is locked; unlock it first.')
LOCK = pathlib.Path('/tmp/pm-shots/gui.lock')
LOCK.parent.mkdir(exist_ok=True)
_deadline = time.monotonic() + 600
while True:
    try:
        with LOCK.open('x') as _f:
            _f.write('saveflight\n')
        break
    except FileExistsError:
        print('Waiting for GUI owner:', LOCK.read_text().strip(), flush=True)
        if time.monotonic() > _deadline:
            sys.exit('GUI lock not acquired; never remove another lane\'s lock')
        time.sleep(30)
close = '--close' in sys.argv
root = pathlib.Path(tempfile.mkdtemp(prefix='pm-native-v2.', dir='/tmp'))
project = root / 'project.json'
mk = lambda i, name: {'id': i, 'type': 'shape', 'name': name, 'from': 0, 'dur': 2, 'on': True,
                      'p': {'opacity': {'v': 100, 'kf': [], 'expr': None}}}
doc = {'id': 'v2', 'name': 'Disposable viewer2', 'compId': 'main', 'w': 320, 'h': 180, 'fps': 30, 'dur': 2,
       'bg': '#000000', 'layers': [mk('base-layer', 'Base layer')] + [mk(f'bulk-{i}', f'Bulk {i}') for i in range(layers)],
       'assets': {}, 'comps': {}}
project.write_text(json.dumps(doc))
profile = root / 'profile'
env = dict(os.environ, PM_PERF_GATE='1', PM_NATIVE_MAIN_TRACE='1')
app = subprocess.Popen([binary, '--profile', str(profile), str(project)], env=env, stdin=subprocess.PIPE,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
events = []  # (t, json)
t0 = time.monotonic()


def pump_err():
    for line in app.stderr:
        if line.startswith('PM_MAIN '):
            try:
                events.append((time.monotonic() - t0, json.loads(line[8:])))
            except ValueError:
                pass
        elif 'warning' not in line.lower():
            sys.stderr.write('app: ' + line)


threading.Thread(target=pump_err, daemon=True).start()
lock = threading.Lock()


def perf(op, **kw):
    with lock:
        app.stdin.write(json.dumps(dict(op=op, **kw)) + '\n'); app.stdin.flush()
        while True:
            line = app.stdout.readline()
            if not line:
                raise RuntimeError('app exited')
            if line.startswith('PM_PERF '):
                return json.loads(line[8:])


class Mcp:
    def __init__(self):
        self.p = subprocess.Popen([binary, '--powermove-mcp', '--profile', str(profile)], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, text=True)
        self.i = 0

    def call(self, method, params):
        self.i += 1
        self.p.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': self.i, 'method': method, 'params': params}) + '\n')
        self.p.stdin.flush()
        r = json.loads(self.p.stdout.readline())
        if 'error' in r or r.get('result', {}).get('isError'):
            raise RuntimeError(f'{method} failed: {json.dumps(r)[:400]}')
        return r

    def tool(self, name, args):
        return self.call('tools/call', {'name': name, 'arguments': args})


SHOTS = pathlib.Path(os.environ.get('SHOTS', '/tmp/pm-viewer2'))
SHOTS.mkdir(parents=True, exist_ok=True)
if not (SHOTS / 'winid').exists():
    subprocess.run(['swiftc', '-O', '-o', str(SHOTS / 'winid'), str(pathlib.Path(__file__).with_name('winid.swift'))], check=True)
TAG = os.environ.get('TAG', 'after')


def shot(name):
    out = subprocess.run([str(SHOTS / 'winid'), str(app.pid)], capture_output=True, text=True).stdout
    ws = [l.split(' ', 6) for l in out.splitlines() if l.strip()]
    ws = [w for w in ws if w[5] == '0']
    if ws:
        w = max(ws, key=lambda w: float(w[3]) * float(w[4]))
        subprocess.run(['screencapture', '-x', '-o', '-l', w[0], str(SHOTS / f'{TAG}-{layers}-{name}.png')])


def phase(name, a, b):
    stalls = [e['stall_ms'] for t, e in events if a <= t <= b and 'stall_ms' in e]
    spans = {}
    for t, e in events:
        if a <= t <= b and 'span' in e:
            spans.setdefault(e['span'], []).append(e['ms'])
    print(f'PHASE {name}: {b - a:.2f}s stalls>=50ms n={len(stalls)} max={max(stalls, default=0):.0f}ms '
          f'spans={ {k: (len(v), round(max(v), 1)) for k, v in spans.items()} }', flush=True)


try:
    line = app.stdout.readline()
    assert 'ready' in line, line
    t = time.monotonic()
    while True:
        s = perf('status')
        if s['opened'] and not s['opening']:
            break
        assert time.monotonic() - t < 180, s
        time.sleep(.25)
    subprocess.run(['osascript', '-e', f'tell application "System Events" to set frontmost of (first process whose unix id is {app.pid}) to true'])
    time.sleep(1)
    shot('1-opened')
    print(f"OPENED {time.monotonic() - t:.1f}s layers={s.get('layer_count')} msg={s.get('message')!r}", flush=True)
    time.sleep(2)
    a = time.monotonic() - t0
    m = Mcp()
    m.call('initialize', {})
    m.tool('start_session', {'prompt': 'Disposable viewer2 check'})
    m.tool('apply_commands', {'commands': [{'type': 'add_layer', 'id': 'flight-layer', 'name': 'Flight edit',
                                            'layerType': 'solid'}]})
    m.tool('finish_session', {'commit': True})
    te = time.monotonic()
    seen = None
    while time.monotonic() - te < 30:
        s = perf('status', layer='flight-layer')
        if s['has_layer']:
            seen = time.monotonic() - te
            break
        time.sleep(.05)
    print(f'MCP EDIT visible in controller: {seen if seen is None else round(seen, 3)}s; dirty={s["dirty"]} '
          f'layers={s["layer_count"]}', flush=True)
    time.sleep(1.5)
    shot('2-after-mcp-edit')
    b = time.monotonic() - t0
    phase('mcp-edit', a, b)
    m.tool('start_session', {'prompt': 'Disposable save'})
    res = {}

    def save():
        t = time.monotonic(); m.tool('save_project', {}); res['save'] = (t, time.monotonic())

    a = time.monotonic() - t0
    th = threading.Thread(target=save); th.start()
    time.sleep(.1)
    seen_saving = False
    msgs = set()
    tc = time.monotonic()
    if close:
        s = perf('close'); msgs.add(s['message'])
    time.sleep(.15)
    shot('3-during-save')
    while th.is_alive():
        s = perf('status'); msgs.add(s['message']); seen_saving |= bool(s['saving'])
        time.sleep(.05)
    th.join()
    s0, s1 = res['save']
    b = time.monotonic() - t0
    print(f'SAVE {s1 - s0:.3f}s; close sent {tc - s0:.3f}s after start; saw saving={seen_saving}; messages={sorted(msgs)}',
          flush=True)
    phase('save', a, b)
    time.sleep(.5)
    shot('4-after-save')
    m.tool('finish_session', {'commit': True})
    m.p.stdin.close(); m.p.wait(timeout=30)
    print('app alive', app.poll() is None, 'status', {k: perf('status')[k] for k in ('message', 'dirty', 'layer_count')})
finally:
    try:
        perf('quit')
    except Exception:
        pass
    try:
        app.wait(timeout=20)
    except subprocess.TimeoutExpired:
        app.kill()
    LOCK.unlink()
    print('root', root)
