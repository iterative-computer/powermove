#!/usr/bin/env python3
"""macOS regression: painted seeks with Properties open bound font data and release it on idle.

Pass --binary and a fresh --out directory outside Git. Copied development binaries
need their matching pm-glsl-compiler sibling and --library-dir for FFmpeg.
Generates a tiny synthetic
project with three animated text faces inside two levels of precompositions.
Uses only an isolated temporary profile, background QoS and PID measurements;
never activates a window, sends OS input, or reads a user's profile/authentication.
No build lock is needed to measure an already built binary. Hold the shared
GUI lock for the full session: lockf -k -t 10800 /tmp/pm-gui.lock python3 <script> …
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import shutil
import signal
import subprocess
import tempfile
import threading
import time


def fixture():
    template = Path(__file__).parents[1] / 'crates/engine-service/tests/fixtures/shapes.json'
    project = json.loads(template.read_text())
    project.update(id='Pfont-memory', name='Native font memory', compId='Pfont-root',
                   compName='Native font memory', w=640, h=360, dur=13.5, work=[0, 13.5],
                   assets={}, comps={}, layers=[])
    prototype = json.loads(template.read_text())['layers'][0]

    def layer(identity, kind, data):
        result = copy.deepcopy(prototype)
        result.update(id=identity, name=identity, type=kind, dur=13.5, d=data)
        result['from'] = 0
        result['p']['position.x']['v'] = 0 if kind == 'precomp' else 320
        result['p']['position.y']['v'] = 0 if kind == 'precomp' else 180
        return result

    def composition(identity, layers):
        # Square child targets keep x/y device density identical after integer
        # target rounding. Nonuniform density routes to outlines, not CanvasMasks.
        return dict(id=identity, name=identity, w=640, h=640, fps=30, dur=13.5,
                    bg='#000000', layers=layers, markers=[])

    for index, font in enumerate(['Arial', 'Times New Roman', 'Courier New']):
        text = layer(f'Ltext{index}', 'text', dict(text='Paint', font=font, size=48,
                     weight=400, color='#D0D0D0', align='center'))
        # Match the scaled nested text's device-space Canvas mask path rather
        # than the legacy 1:1 raster shortcut.
        for axis in ['scale.x', 'scale.y']:
            text['p'][axis]['v'] = 60
        # Animate the text size itself. A legacy precomp may resample an
        # already-rasterized layer when only its transform changes; that does
        # not exercise the native per-strike font map.
        text['d']['size'] = dict(v=20, expr=None, kf=[
            dict(t=0, v=20, i=f'k{index}a', inInterp='linear', outInterp='linear'),
            dict(t=13.5, v=50, i=f'k{index}b', inInterp='linear', outInterp='linear')])
        inner, outer = f'Cinner{index}', f'Couter{index}'
        project['comps'][inner] = composition(inner, [text])
        project['comps'][outer] = composition(outer, [layer(f'Lnested{index}', 'precomp',
                                                    dict(comp=inner, w=640, h=640))])
        project['layers'].append(layer(f'Lselect{index}', 'precomp', dict(comp=outer, w=640, h=640)))
    return dict(v='1.0.0', proj=project,
                history=dict(version=1, paths='composition', index=-1, entries=[]))


class Viewer:
    def __init__(self, binary, profile, project, out, library_dir=None):
        self.binary, self.profile, self.out = binary, profile, out
        self.library_env = ({'DYLD_LIBRARY_PATH': str(library_dir.resolve(strict=True))}
                            if library_dir else {})
        self.processes, self.files = [], []
        self.seq = 0
        self.viewer, self.frames = self.start(
            [str(project)], 'viewer', dict(MallocLargeCache='0', MallocStackLogging='1',
                                         PM_PERF_GATE='1', PM_NATIVE_NO_ORIGINAL='1'), 'PM_PERF ')

    def start(self, arguments, name, extra, prefix=''):
        log = (self.out / f'{name}-stderr.log').open('w')
        self.files.append(log)
        # taskpolicy is protected by SIP and strips DYLD_* from its own
        # environment. Apply the development library path after that wrapper.
        launch = ['taskpolicy', '-b']
        if self.library_env:
            launch += ['/usr/bin/env', *[f'{key}={value}'
                                        for key, value in self.library_env.items()]]
        process = subprocess.Popen(
            [*launch, str(self.binary), '--profile', str(self.profile), *arguments],
            env={**os.environ, **self.library_env, **extra},
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True)
        self.processes.append(process)
        (self.out / f'{name}-pid').write_text(str(process.pid))
        replies = queue.Queue()

        def read():
            for line in process.stdout:
                if line.startswith(prefix):
                    try:
                        replies.put(json.loads(line[len(prefix):]))
                    except json.JSONDecodeError:
                        continue
        threading.Thread(target=read, daemon=True).start()
        return process, replies

    def perf(self, **request):
        self.viewer.stdin.write(json.dumps(request) + '\n')
        self.viewer.stdin.flush()
        return self.frames.get(timeout=60)

    def rpc(self, method, params):
        self.seq += 1
        self.relay.stdin.write(json.dumps(dict(jsonrpc='2.0', id=self.seq,
                                              method=method, params=params)) + '\n')
        self.relay.stdin.flush()
        while True:
            response = self.replies.get(timeout=60)
            if response.get('id') == self.seq:
                if response.get('error') or response.get('result', {}).get('isError'):
                    raise RuntimeError(str(response))
                return response['result']

    def tool(self, name, **arguments):
        return self.rpc('tools/call', dict(name=name, arguments=arguments))

    def agent(self):
        self.relay, self.replies = self.start(['--powermove-mcp'], 'relay',
                                             dict(PM_NATIVE_PROFILE=str(self.profile)))
        self.rpc('initialize', dict(protocolVersion='2024-11-05', capabilities={},
                 clientInfo=dict(name='painted-selection-memory', version='1')))
        self.tool('start_session', access='project', prompt='Measure generated font fixture only')
        for attempt in range(3):
            try:
                panel = self.tool('open_panel', panelId='inspector')
                break
            except RuntimeError as error:
                if 'The panel did not finish mounting' not in str(error) or attempt == 2:
                    raise
                self.perf(op='redraw')
                (self.out / 'panel-mount-retries.txt').write_text(str(attempt + 1))
                time.sleep(.25)
        (self.out / 'properties.json').write_text(json.dumps(panel, indent=2))
        content = json.loads(panel['content'][0]['text'])
        bounds = content.get('bounds', {})
        assert bounds.get('width', 0) > 0 and bounds.get('height', 0) > 0, 'Properties must be visible'

    def paint(self, seconds):
        previous = self.perf(op='status').get('frame') or {}
        self.perf(op='seek', seconds=seconds)
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            self.perf(op='redraw')
            state = self.perf(op='status')
            frame = state.get('frame') or {}
            if (frame.get('gpu_ok') and frame.get('stamp') != previous.get('stamp')
                    and abs(frame.get('ticks', -100) - round(seconds * 705600000)) <= 1):
                assert not state.get('window_key'), 'The isolated viewer must not take focus'
                return state
            time.sleep(.1)
        (self.out / 'failed-paint.json').write_text(json.dumps(state, indent=2))
        raise RuntimeError('No fresh painted frame for seek')

    def capture(self, label):
        for kind, command in [('footprint', ['footprint', '-p']), ('heap', ['heap', '-s'])]:
            result = subprocess.run([*command, str(self.viewer.pid)], capture_output=True,
                                    text=True, timeout=60, check=True)
            (self.out / f'{label}-{kind}.txt').write_text(result.stdout + result.stderr)
            if kind == 'heap':
                match = re.search(r'^\s*\d+\s+(\d+)\s+[\d.]+\s+CFData \(Bytes Storage\)',
                                  result.stdout, re.M)
                if not match:
                    raise RuntimeError('heap did not report live CFData storage')
                live = int(match[1])
        return dict(step=label, cfdata_bytes=live)

    def close(self):
        for process in reversed(self.processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        for log in self.files:
            log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--library-dir', type=Path,
                        help='FFmpeg lib directory for a copied development binary')
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--cycles', type=int, default=50)
    args = parser.parse_args()
    if not 50 <= args.cycles <= 300:
        parser.error('Use 50 to 300 cycles')
    out = args.out.expanduser().resolve()
    repo = Path(__file__).resolve().parents[3]
    if out.is_relative_to(repo):
        parser.error('Private measurements must stay outside Git')
    out.mkdir(parents=True, exist_ok=False)
    lock = Path('/tmp/pm-screen.lock.d')
    lock.mkdir()
    (lock / 'owner').write_text(f'painted-selection-memory {os.getpid()}')
    profile = Path(tempfile.mkdtemp(prefix='pm-font-memory-'))
    viewer = None
    try:
        project = profile / 'synthetic.pmv'
        project.write_text(json.dumps(fixture()))
        (out / 'fixture.pmv').write_bytes(project.read_bytes())
        (out / 'fixture-sha256.txt').write_text(hashlib.sha256(project.read_bytes()).hexdigest() + '\n')
        viewer = Viewer(args.binary.resolve(), profile, project, out, args.library_dir)
        ready = viewer.frames.get(timeout=120)
        assert ready.get('ready'), ready
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            state = viewer.perf(op='status')
            if state.get('opened') and not state.get('opening'):
                break
            time.sleep(.5)
        else:
            raise RuntimeError('Project did not open')
        viewer.agent()
        viewer.paint(1 / 30)
        rows = [viewer.capture('open')]
        for cycle in range(args.cycles):
            viewer.tool('select_layers', layerIds=[f'Lselect{cycle % 3}'])
            state = viewer.paint((5 + cycle * (360 // args.cycles)) / 30)
            rows.append(viewer.capture(f'painted-{cycle + 1}'))
            with (out / 'painted.jsonl').open('a') as log:
                log.write(json.dumps(state) + '\n')
        viewer.tool('finish_session', commit=True)
        rows.append(viewer.capture('finish'))
        time.sleep(8)
        rows.append(viewer.capture('idle'))
        (out / 'summary.json').write_text(json.dumps(rows, indent=2) + '\n')
        baseline = rows[0]['cfdata_bytes']
        # Process-wide CFData includes font registry and UI storage. The native
        # providers have a 32 MiB budget; allow another 32 MiB for other CFData.
        bounded = max(r['cfdata_bytes'] for r in rows) <= baseline + (64 << 20)
        # Idle must release the renderer's provider copies, not just stop growth.
        released = rows[-1]['cfdata_bytes'] <= baseline + (8 << 20)
        checks = dict(bounded=bounded, released_on_idle=released)
        (out / 'checks.json').write_text(json.dumps(checks, indent=2) + '\n')
        assert bounded and released, f'CFData regression: {checks}'
        print('PASS: painted CFData bounded and returned on idle')
    finally:
        if viewer:
            viewer.close()
        try:
            # A CLI-runtime installer may finish its last write just after the
            # isolated app exits. Retry removal of only this generated profile.
            for attempt in range(5):
                try:
                    shutil.rmtree(profile)
                    break
                except OSError:
                    if attempt == 4:
                        raise
                    time.sleep(.2)
        finally:
            (lock / 'owner').unlink()
            lock.rmdir()


if __name__ == '__main__':
    def interrupted(_signal, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    main()
