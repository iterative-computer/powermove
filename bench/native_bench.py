#!/usr/bin/env python3
"""Powermove Native benchmark driver (one run of one scene).

GUI metrics use the app's own opt-in perf gate (PM_PERF_GATE=1: newline JSON over stdin/stdout,
documented in tools/perf-gate) and its engine trace (PM_ENGINE_TRACE, one JSON row per frame).
Exports use the app's headless MCP host (--powermove-mcp, export_video) on the same profile.
Usage: native_bench.py --bin <powermove-native> --scene <scene.pmv> --out <dir> --run N [--export]
Emits "BENCH {...}" lines like electron_bench.mjs. Profiles and project copies live under /tmp
(the perf gate refuses anything else).
"""
import argparse, json, os, random, select, shutil, subprocess, tempfile, threading, time
TICKS = 705_600_000
emit = lambda **v: print('BENCH ' + json.dumps(v), flush=True)

def family(pid):
    out, todo = {pid}, [pid]
    while todo:
        p = todo.pop()
        kids = subprocess.run(['pgrep', '-P', str(p)], capture_output=True, text=True).stdout.split()
        for k in map(int, kids):
            if k not in out: out.add(k); todo.append(k)
    return out

def rss_mb(pid):
    pids = family(pid)
    r = subprocess.run(['ps', '-o', 'rss=', '-p', ','.join(map(str, pids))], capture_output=True, text=True).stdout.split()
    return sum(map(int, r)) / 1024 if r else None

class Sampler:
    def __init__(self, pid): self.pid, self.peak, self.n, self.stop = pid, 0., 0, False; self.t = threading.Thread(target=self.loop); self.t.start()
    def loop(self):
        while not self.stop:
            v = rss_mb(self.pid)
            if v: self.peak = max(self.peak, v); self.n += 1
            time.sleep(.2)
    def done(self): self.stop = True; self.t.join(); return {'peak_mb': self.peak, 'samples': self.n}

class App:
    def __init__(self, binary, profile, trace):
        env = dict(os.environ, PM_PERF_GATE='1', TMPDIR='/tmp', PM_ENGINE_TRACE=trace)
        self.t0 = time.monotonic()
        self.p = subprocess.Popen([binary, '--profile', profile], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, env=env, bufsize=0)
        self.buf = b''
        self.ready = self.read(60)
        self.pid = self.ready['pid']
        while not self.req('status').get('window_visible'): time.sleep(.01)
        self.launch_s = time.monotonic() - self.t0
    def read(self, sec=30):
        end = time.monotonic() + sec
        while time.monotonic() < end:
            while b'\n' in self.buf:
                line, self.buf = self.buf.split(b'\n', 1)
                if line.startswith(b'PM_PERF '):
                    v = json.loads(line[8:])
                    if v.get('error'): raise RuntimeError(v['error'])
                    return v
            if select.select([self.p.stdout], [], [], .5)[0]:
                c = os.read(self.p.stdout.fileno(), 65536)
                if not c: raise RuntimeError('native exited')
                self.buf += c
        raise TimeoutError('perf gate timeout')
    def req(self, op, **k):
        self.p.stdin.write((json.dumps(dict(op=op, **k)) + '\n').encode()); return self.read()
    def quit(self):
        try: self.p.stdin.write(b'{"op":"quit"}\n'); self.p.wait(15)
        except Exception: self.p.kill()

def wait_frame(app, accepted, ticks=None, viewer=None, timeout=60):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        s = app.req('status'); f = s.get('frame')
        if f and f.get('gpu_ok') and s.get('opened') and not s.get('opening') and f.get('completion_host_s') and f['completion_host_s'] >= accepted \
           and (ticks is None or abs(f['ticks'] - ticks) <= 1) and (viewer is None or s.get('viewer_id') != viewer):
            return f, s
        time.sleep(.002)
    raise TimeoutError('no matching GPU-complete frame')

def mcp_export(binary, profile, name, fmt, outdir, prores_profile='422', dest=None):
    """Open the library copy of the project in the headless MCP host and export it."""
    p = subprocess.Popen([binary, '--powermove-mcp', '--profile', profile], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
    n = [0]
    def call(method, params=None):
        n[0] += 1; p.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': n[0], 'method': method, 'params': params or {}}) + '\n')
        while True:
            m = json.loads(p.stdout.readline())
            if m.get('id') == n[0]: return m
    def tool(name, args):
        r = call('tools/call', {'name': name, 'arguments': args})
        if 'error' in r or r['result'].get('isError'): raise RuntimeError(json.dumps(r)[:600])
        return json.loads(r['result']['content'][0]['text'])
    try:
        call('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'bench', 'version': '1'}})
        p.stdin.write(json.dumps({'jsonrpc': '2.0', 'method': 'notifications/initialized'}) + '\n')
        projects = tool('list_projects', {})
        items = projects.get('projects', projects if isinstance(projects, list) else [])
        pid = items[0]['id'] if len(items) == 1 else next(x['id'] for x in items if x.get('name') == name)
        tool('open_project', {'projectId': pid})
        tool('start_session', {'prompt': 'Benchmark export', 'access': 'project', 'context': 'project'})
        args = {'format': fmt, 'range': 'all', 'audio': True, 'name': f'{name}-{fmt}'}
        if fmt == 'prores': args['profile'] = prores_profile
        samp = Sampler(p.pid); t0 = time.monotonic()
        res = tool('export_video', args)
        while res.get('status') == 'running': res = tool('export_video', {'jobId': res['jobId']})
        el, mem = time.monotonic() - t0, samp.done()
        path = res.get('path')
        if path and dest: os.makedirs(os.path.dirname(dest), exist_ok=True); shutil.move(path, dest); path = dest
        return {'export_s': el, 'path': path, 'result': {k: v for k, v in res.items() if k != 'path'}, 'mem': mem}
    finally:
        p.stdin.close(); p.terminate(); p.wait(10)

def main():
    a = argparse.ArgumentParser(); a.add_argument('--bin', required=True); a.add_argument('--scene', required=True)
    a.add_argument('--out', required=True); a.add_argument('--run', type=int, default=0); a.add_argument('--export', action='store_true')
    a = a.parse_args()
    work = tempfile.mkdtemp(prefix='pm-bench-native-', dir='/tmp')
    profile, trace = os.path.join(work, 'profile'), os.path.join(work, 'trace.jsonl')
    scene = os.path.join(work, os.path.basename(a.scene)); shutil.copy(a.scene, scene)
    app = App(a.bin, profile, trace)
    emit(op='launch', run=a.run, launch_s=app.launch_s, endpoint='spawn->window_visible (controller ready)')
    before = app.req('status').get('viewer_id')
    t = app.req('open', path=scene)
    f, s = wait_frame(app, t['accepted_host_s'], viewer=before, timeout=120)
    emit(op='open', run=a.run, scene=a.scene, open_s=f['completion_host_s'] - t['accepted_host_s'], endpoint='open accepted->first GPU-complete frame',
         physical_present=f.get('display_host_s') is not None, name=s.get('shell', {}).get('title'))
    time.sleep(3)
    emit(op='idle_rss', run=a.run, scene=a.scene, rss_mb=rss_mb(app.pid), metal_allocated_mb=app.req('status')['metal_allocated_bytes'] / 2**20)
    # Playback: 10 s from frame 0; frames counted from the engine trace (GPU-complete content frames).
    t = app.req('seek', seconds=0); wait_frame(app, t['accepted_host_s'], ticks=0); time.sleep(1)
    mark = os.path.getsize(trace) if os.path.exists(trace) else 0
    samp = Sampler(app.pid)
    t = app.req('play'); start = t['accepted_host_s']; time.sleep(10); stop = app.req('pause')['accepted_host_s']
    mem = samp.done()
    rows = []
    with open(trace) as fh:
        fh.seek(mark)
        for line in fh:
            try: r = json.loads(line)
            except ValueError: continue
            if r.get('gpu_ok') is True and r.get('moment') == 'Playing' and r.get('completed') and start <= r['completed'] <= stop: rows.append(r)
    distinct = len({r['time'] for r in rows}); el = stop - start; expected = round(el * 30)
    emit(op='playback', run=a.run, scene=a.scene, elapsed_s=el, distinct_frames=distinct, fps=distinct / el, dropped=max(0, expected - distinct), expected=expected,
         presented_rows=sum(1 for r in rows if r.get('display')), mem=mem)
    rnd = random.Random(12345); lat = []
    for _ in range(60):
        frame = rnd.randrange(300); ticks = frame * TICKS // 30
        t = app.req('seek', seconds=frame / 30)
        f, _ = wait_frame(app, t['accepted_host_s'], ticks=ticks)
        lat.append((f['completion_host_s'] - t['accepted_host_s']) * 1000)
    srt = sorted(lat)
    emit(op='scrub', run=a.run, scene=a.scene, median_ms=srt[30], p95_ms=srt[56], samples_ms=lat, endpoint='seek accepted->GPU-complete')
    app.quit()
    if a.export:
        stem = os.path.splitext(os.path.basename(a.scene))[0]
        for fmt, pp, ext in [('mp4', None, 'mp4'), ('prores', '422', 'mov'), ('prores', '4444', 'mov')]:
            label = fmt if fmt == 'mp4' else 'prores' + pp
            dest = os.path.join(a.out, f'{stem}-native-{label}-r{a.run}.{ext}')
            try: emit(op='export', run=a.run, scene=a.scene, format=label, **mcp_export(a.bin, profile, stem, fmt, a.out, pp or '422', dest))
            except Exception as e: emit(op='export', run=a.run, scene=a.scene, format=label, error=repr(e)[:600])
    shutil.rmtree(work, ignore_errors=True)

if __name__ == '__main__': main()
