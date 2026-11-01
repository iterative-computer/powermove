#!/usr/bin/env python3
"""Paired P1 measurements. Raw machine/process details remain under /tmp.

No headless engine timings, render events, missing counters or RSS are promoted
to physical display, A/V, energy, or deduplicated family-memory gate results.
"""
import argparse
from contextlib import contextmanager
import json
import hashlib
import os
from pathlib import Path
import platform
import plistlib
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import time

from budgets import ROWS, check, distribution, ratio
from macos import Processes, delta
from trace import load as load_trace

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
LOCK = Path('/tmp/pm-shots/gui.lock')
TICKS = 705_600_000
REASONS = {
    'cold_launch': 'Controller/harness readiness is not first interactive physical frame; filesystem caches are not purged.',
    'open': 'Native file/kernel open and Electron corpus-event open have different boundaries; physical Electron presentation and native main-thread stalls are unavailable.',
    'scrub_cached': 'Native exposes drawable presentation but Electron exposes GPU-complete plus next rAF only; input is an automation command, not a timestamped hardware event.',
    'scrub_random_seek': 'Prepared workload is an unaltered public shape composition, not 4K long-GOP; no substituted workload can pass this row.',
    'playback': 'Native trace is retained but does not guarantee a complete frame census; no paired Electron physical presentation or per-frame work stream.',
    'playback_cost': 'Per-process CPU time is observable; native public status omits Metal GPU-time counters, and no paired app-attributed energy counter is exposed.',
    'av': 'Native matched-device trace samples are retained when present; the frozen Electron adapter lacks matched device presentation stamps for a paired gate.',
    'edit_repaint': 'Viewer-frame change does not prove shell repaint; Electron physical presentation and native per-frame main-thread work are unavailable.',
    'idle': 'Family CPU and kernel interrupt/package wakeups are observable; kernel counters do not attribute wakeups to individual extensions.',
    'memory_small': 'Summed per-process phys_footprint is recorded separately from Metal allocated size; shared CPU/GPU allocation deduplication and document/extension splits are unavailable.',
}


class BatchExpired(BaseException):
    """Must bypass per-row recoverable errors so a batch cannot outlive its lock."""


def command(args, **kwargs):
    return subprocess.run(args, text=True, capture_output=True, timeout=kwargs.pop('timeout', 20), **kwargs)


def power():
    r = command(['pmset', '-g', 'batt'])
    return r.returncode == 0 and "'AC Power'" in r.stdout


def screen_unlocked():
    # Read only lock booleans; never retain console-user identities or attempt unlock.
    r = subprocess.run(['ioreg', '-a', '-n', 'Root', '-d', '1'], capture_output=True, timeout=10)
    flags = []
    def walk(value):
        if isinstance(value, dict):
            for key, item in value.items():
                if key in {'IOConsoleLocked', 'CGSSessionScreenIsLocked'} and isinstance(item, bool):
                    flags.append(item)
                walk(item)
        elif isinstance(value, list):
            for item in value: walk(item)
    try:
        walk(plistlib.loads(r.stdout))
    except (plistlib.InvalidFileException, ValueError):
        return False
    return bool(flags) and not any(flags)


def conditions(proc, *, quiet=True):
    load = os.getloadavg()
    state = {'load': load, 'ac_power': power(), 'screen_unlocked':screen_unlocked(), 'free_bytes': shutil.disk_usage('/').free,
             'os_build': command(['sw_vers', '-buildVersion']).stdout.strip()}
    blockers = []
    if state['free_bytes'] < 8 * 1024**3:
        blockers.append('Less than 8 GiB free; builds and runs stopped')
    if not state['ac_power']:
        blockers.append('Machine is not on AC power')
    if not state['screen_unlocked']:
        blockers.append('Mac session is locked or its state cannot be verified; no on-screen run')
    if load[0] >= 12:
        blockers.append('One-minute load is not below 12')
    if quiet:
        apps = proc.other_apps()
        state['other_app_count'] = len(apps)
        if apps:
            blockers.append('Other Powermove/Electron instances are active; leave them untouched')
        builds = [p for p in proc.all() if Path(proc.path(p)).name in {'rustc', 'cargo', 'clang', 'clang++'}]
        state['build_process_count'] = len(builds)
        if builds:
            blockers.append('Build processes are active; machine is not settled')
    presence_names = {'Finder','Aside','Grok Bot','Notion','Mail','Claude','T3 Code (Nightly)','Powermove','RobloxPlayer','RobloxMenuBar','figma_agent','OBS','Paper'}
    state['app_presence'] = sorted({Path(proc.path(pid)).name for pid in proc.all()} & presence_names)
    state['blockers'] = blockers
    return state


def execution_blockers(state, diagnostic=False):
    allowed = {'One-minute load is not below 12', 'Other Powermove/Electron instances are active; leave them untouched'}
    return [reason for reason in state['blockers'] if not diagnostic or reason not in allowed]


def machine():
    def sysctl(key):
        return command(['sysctl', '-n', key]).stdout.strip()
    # No serial numbers, host names, account names, profile paths or corpus ids.
    state = {'model': sysctl('hw.model'), 'cpu': sysctl('machdep.cpu.brand_string'),
             'ram_bytes': int(sysctl('hw.memsize')), 'os': platform.mac_ver()[0],
             'os_build': command(['sw_vers', '-buildVersion']).stdout.strip(),
             'required_rigs_certified': False}
    probe = command(['powermetrics', '-n', '1', '-i', '100', '--samplers', 'cpu_power,gpu_power'], timeout=5)
    state['powermetrics'] = {'available_without_sudo': probe.returncode == 0,
                            'reason': None if probe.returncode == 0 else probe.stderr.strip()[:300]}
    # A system-wide sample does not establish process-attributed energy, even when permitted.
    return state


@contextmanager
def gui_lock(wait_seconds=300, session_inode=None):
    if session_inode is not None:
        if not LOCK.exists() or LOCK.stat().st_ino != session_inode or LOCK.read_text().strip() != "onscreen":
            raise RuntimeError("Session GUI lock identity changed")
        yield
        return
    LOCK.parent.mkdir(parents=True, exist_ok=True)
    deadline = time.monotonic() + wait_seconds
    while True:
        result = command(['zsh', '-c', 'set -o noclobber; echo perf > /tmp/pm-shots/gui.lock'])
        if result.returncode == 0:
            break
        if time.monotonic() + 30 > deadline:
            raise RuntimeError('GUI lock held; never remove another lane\'s lock')
        print('GUI lock held; retry in 30 s', flush=True)
        time.sleep(30)
    inode = LOCK.stat().st_ino
    def expired(_signum, _frame):
        raise BatchExpired('GUI batch exceeded 9 minutes; tearing down owned app')
    previous = signal.signal(signal.SIGALRM, expired)
    signal.alarm(540)
    try:
        yield
    finally:
        signal.alarm(0)
        signal.signal(signal.SIGALRM, previous)
        if LOCK.exists() and LOCK.stat().st_ino == inode:
            LOCK.unlink()


class App:
    def __init__(self, kind, args, scratch):
        self.kind, self.scratch = kind, scratch
        self.log = open(args.output.parent / (scratch.name + '-' + kind + '.stderr.log'), 'wb')
        # Rust and Node must agree with the explicit /tmp corpus/profile roots,
        # even when macOS supplies a per-account /var/folders TMPDIR.
        env = dict(os.environ, PM_PERF_GATE='1', TMPDIR='/tmp')
        self.trace_path = args.output.parent / (scratch.name + '-native.trace.json')
        if kind == 'native':
            env['PM_ENGINE_TRACE'] = str(self.trace_path)
        else:
            env.pop('PM_ENGINE_TRACE', None)
        env.pop('ELECTRON_RUN_AS_NODE', None)
        env.pop('POWERMOVE_E2E_LIVE', None)
        argv = ([str(args.native), '--profile', str(scratch / 'native-profile')] if kind == 'native'
                else ['node', str(HERE / 'electron.mjs'), str(args.reference)])
        self.started = time.monotonic()
        self.process = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.log, env=env, start_new_session=True, bufsize=0)
        self.buffer = b''
        self.pid = None
        try:
            self.ready = self.read(45)
            self.ready_ms = (time.monotonic() - self.started) * 1000
            if not self.ready.get('ready'):
                raise RuntimeError('Application did not report readiness')
            self.pid = int(self.ready['pid'])
        except BaseException:
            self.close()
            raise

    def read(self, seconds=20):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            while b'\n' in self.buffer:
                line, self.buffer = self.buffer.split(b'\n', 1)
                if line.startswith(b'PM_PERF '):
                    value = json.loads(line[8:])
                    if value.get('error'):
                        raise RuntimeError(str(value['error']))
                    return value
            left = deadline - time.monotonic()
            if left > 0 and select.select([self.process.stdout], [], [], left)[0]:
                chunk = os.read(self.process.stdout.fileno(), 65536)
                if not chunk:
                    break
                self.buffer += chunk
        raise TimeoutError(f'{self.kind}: measurement protocol stopped or timed out')

    def request(self, op, **kwargs):
        self.process.stdin.write((json.dumps(dict(op=op, **kwargs)) + '\n').encode())
        self.process.stdin.flush()
        return self.read()

    def close(self):
        if self.process.poll() is None:
            try:
                self.process.stdin.write(b'{"op":"quit"}\n')
                self.process.stdin.close()
                self.process.wait(timeout=8)
            except (BrokenPipeError, subprocess.TimeoutExpired, OSError):
                pass
            if self.process.poll() is None:
                os.killpg(self.process.pid, signal.SIGTERM)
                try:
                    self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    os.killpg(self.process.pid, signal.SIGKILL)
                    self.process.wait(timeout=3)
        self.log.close()
        # Reap any remaining members of this run's unique process group only.
        try:
            os.killpg(self.process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass

    def action(self, op, **kwargs):
        if self.kind == 'electron':
            return self.request(op, **kwargs)
        old = self.request('status')
        start = self.request(op, **kwargs)
        accepted = start['accepted_host_s']
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            state = self.request('status')
            frame = state.get('frame')
            matches = bool(frame and frame.get('gpu_ok') and frame.get('display_host_s') and
                           frame['display_host_s'] >= accepted)
            if op == 'seek':
                # Caller supplies an exact project frame time.
                matches = matches and frame.get('input_host_s') == accepted
                matches = matches and abs(frame['ticks'] - round(kwargs['seconds'] * TICKS)) <= 1
            if op == 'edit':
                matches = matches and frame.get('input_host_s') == accepted
                matches = matches and frame.get('revision', 0) > old.get('revision', 0)
                matches = matches and frame['stamp'] != (old.get('frame') or {}).get('stamp')
            if op == 'open':
                matches = matches and state.get('viewer_id') != old.get('viewer_id')
            if matches and state.get('opened') and not state.get('opening'):
                return {'ms': (frame['display_host_s'] - accepted) * 1000,
                        'endpoint': 'command-accepted-to-drawable-presentedTime',
                        'physical_present': True, 'cache_hit': frame['cache_hit'],
                        'metal_allocated_bytes': state['metal_allocated_bytes']}
            time.sleep(.002)
        raise TimeoutError('No matching native physically presented frame; missing stays missing')


def observe(app, proc, seconds):
    start = proc.family(app.pid)
    began = time.monotonic()
    time.sleep(seconds)
    end = proc.family(app.pid)
    elapsed = time.monotonic() - began
    return dict(delta(start, end, elapsed), duration_s=elapsed,
                per_process=[{'pid':pid,'phys_footprint_bytes':u['phys_footprint']} for pid,u in end.items()])


def position(app, seconds):
    # Setup seeks to an already displayed frame do not require another drawable.
    # Never use this shortcut for the measured cached seek itself.
    if app.kind == 'native':
        frame = app.request('status').get('frame')
        if frame and abs(frame['ticks'] - round(seconds * TICKS)) <= 1:
            return
    app.action('seek', seconds=seconds)


def one_app(kind, args, proc, scratch, run):
    values, errors = {}, {}
    app = App(kind, args, scratch)
    try:
        values['cold_launch'] = {'ready_ms': app.ready_ms, 'endpoint': app.ready['endpoint']}
        base = proc.family(app.pid)
        base_mb = sum(p['phys_footprint'] for p in base.values()) / 1e6
        try:
            project = scratch / (kind + '-document.json')
            shutil.copyfile(args.corpus / 'document.json', project)
            values['open'] = app.action('open', path=str(project))
        except Exception as error:
            errors['open'] = str(error)
            return values, errors
        time.sleep(args.settle_seconds)
        for row in ['scrub_cached', 'edit_repaint']:
            try:
                if row == 'scrub_cached':
                    for t in [0., 1 / args.fps, 0.]:
                        position(app, t)
                    values[row] = app.action('seek', seconds=1 / args.fps)
                    if kind == 'native' and values[row].get('cache_hit') is not True:
                        raise RuntimeError('Native frame was not a verified cache hit')
                else:
                    position(app, 0.)
                    values[row] = app.action('edit', value=37 if run % 2 == 0 else 73)
            except Exception as error:
                values.pop(row, None)
                errors[row] = str(error)
        app.request('pause')
        time.sleep(args.settle_seconds)
        values['idle'] = observe(app, proc, args.idle_seconds)
        values['memory_small'] = {'family_phys_footprint_mb': values['idle']['family_phys_footprint_mb'],
                                  'base_family_phys_footprint_mb':base_mb,
                                  'document_family_delta_mb':values['idle']['family_phys_footprint_mb']-base_mb,
                                  'extension_allocation_split':None,
                                  'per_process':values['idle']['per_process'],
                                  'method': 'sum of per-process phys_footprint; shared-allocation deduplication unverified'}
        # Full 60 s intervals. Limited repeats because the missing GPU/display
        # streams already prevent these rows passing; no short interval is called playback.
        if run < args.playback_runs:
            try:
                position(app, 0.)
                app.request('play')
                values['playback_cost'] = observe(app, proc, 60)
                if not app.request('status').get('playing'):
                    values.pop('playback_cost', None)
                    raise RuntimeError('Transport stopped during playback interval; reject CPU-only interval')
                app.request('pause')
                values['playback'] = {'duration_s': values['playback_cost']['duration_s'],
                                      'endpoint': 'CPU-accounting window; physical frame census unavailable'}
            except Exception as error:
                errors['playback_cost'] = str(error)
    finally:
        app.close()
        if kind == 'native' and app.trace_path.exists():
            try:
                values['service_trace'] = load_trace(app.trace_path)
            except (ValueError, OSError) as error:
                errors['service_trace'] = str(error)
    return values, errors


def summary(data, *, diagnostic=False):
    rows = {}
    for row in ROWS:
        sides = {}
        for kind in ['native', 'electron']:
            observations = [p[kind].get(row, {}) for p in data['pairs'] if (p.get('valid_conditions') or diagnostic) and kind in p]
            keys = sorted({key for o in observations for key, value in o.items() if isinstance(value, (int, float)) and not isinstance(value, bool) and not key.endswith('_delta_mb')})
            sides[kind] = {key: [o[key] for o in observations if key in o] for key in keys}
        # These measurements have explicitly different or unavailable gate endpoints.
        # Only CPU is consumed by its own clause; diagnostic latency remains named 'ms'.
        outcome = check(row, sides['native'], sides['electron'], hz=data['refresh_hz'], runs=30,
                        conditions=data['machine']['required_rigs_certified'], endpoints=False)
        stats = {kind: {key: distribution(vals) for key, vals in side.items()} for kind, side in sides.items()}
        ratios = {key: {q: ratio(stats['native'][key][q], stats['electron'][key][q]) for q in ['p50', 'p95']}
                  for key in stats['native'].keys() & stats['electron'].keys()}
        rows[row] = dict(outcome, stats=stats, ratios=ratios, budget=ROWS[row], reason=REASONS[row])
    return rows


def run(args):
    proc = Processes()
    data = {'schema': 'powermove.perf-gate.p1/v1', 'machine': machine(), 'refresh_hz': args.refresh_hz,
            'native_revision': command(['git', '-C', str(ROOT), 'rev-parse', 'HEAD']).stdout.strip(),
            'reference_revision': command(['git', '-C', str(args.reference), 'rev-parse', 'HEAD']).stdout.strip(),
            'reference_tag': 'v1.2.0', 'requested_runs': args.runs, 'pairs': [],
            'diagnostic_mode': getattr(args, 'diagnostic', False),
            'playback_runs': args.playback_runs, 'playback_repeat_reason': 'Physical frame census, GPU and app-attributed energy are unavailable; extra CPU-only repeats cannot satisfy the combined rows.',
            'initial_conditions': conditions(proc), 'blockers': []}
    data['blockers'].extend(data['initial_conditions']['blockers'])
    try:
        if execution_blockers(data['initial_conditions'], getattr(args, "diagnostic", False)):
            return data
        expected = command(['git', '-C', str(ROOT), 'rev-parse', 'v1.2.0^{commit}']).stdout.strip()
        if data['reference_revision'] != expected or not expected:
            raise RuntimeError('Reference checkout is not the frozen v1.2.0 tag')
        if command(['git', '-C', str(args.reference), 'diff', '--quiet', 'HEAD', '--']).returncode:
            raise RuntimeError('Frozen reference has tracked source changes')
        fixture = json.loads((args.corpus / 'fixture.json').read_text())
        if fixture.get('source') != 'engine/corpus' or hashlib.sha256((args.corpus / 'document.json').read_bytes()).hexdigest() != fixture.get('sha256'):
            raise RuntimeError('Public corpus fixture metadata/hash mismatch')
        args.fps = fixture['fps']
        data['native_dirty'] = bool(command(['git', '-C', str(ROOT), 'status', '--porcelain', '--untracked-files=no']).stdout.strip())
        data['native_binary_sha256'] = hashlib.sha256(args.native.read_bytes()).hexdigest()
        data['fixture'] = fixture
        for iteration in range(args.runs):
            pair = {'index': iteration, 'order': ['native', 'electron'] if iteration % 2 == 0 else ['electron', 'native'], 'conditions': [], 'errors': {}}
            with gui_lock(args.lock_wait_seconds, getattr(args, "session_lock_inode", None)):
                state = conditions(proc)
                if execution_blockers(state, getattr(args, "diagnostic", False)):
                    data['blockers'].extend(state['blockers'])
                    break
                with tempfile.TemporaryDirectory(prefix='pm-perf-pair-', dir='/tmp') as scratch:
                    for kind in pair['order']:
                        before = conditions(proc)
                        pair['conditions'].append(before)
                        if execution_blockers(before, getattr(args, "diagnostic", False)):
                            raise RuntimeError('; '.join(before['blockers']))
                        try:
                            pair[kind], pair['errors'][kind] = one_app(kind, args, proc, Path(scratch), iteration)
                        except Exception as error:
                            pair['errors'][kind] = {'session': str(error)}
                        after = conditions(proc)
                        pair['conditions'].append(after)
                        if after['os_build'] != data['machine']['os_build']:
                            after['blockers'].append('OS build changed during run')
                pair['valid_conditions'] = not any(c['blockers'] for c in pair['conditions'])
            data['pairs'].append(pair)
            args.output.write_text(json.dumps(data, indent=2))
            print(f'Pair {iteration+1}/{args.runs}: {"valid" if pair["valid_conditions"] else "rejected conditions"}; errors={bool(any(pair["errors"].values()))}', flush=True)
            # Broken measurement seams should not produce 30 copies of the same error.
            if any('session' in e or 'open' in e for e in pair['errors'].values()):
                data['blockers'].append('Application launch/open seam failed; inspect private logs and fix before repetition')
                break
    except Exception as error:
        data['blockers'].append(str(error))
    finally:
        data['results'] = summary(data)
        data['diagnostic_results'] = summary(data, diagnostic=True) if getattr(args, "diagnostic", False) else {}
        args.output.write_text(json.dumps(data, indent=2))
    return data


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--diagnostic', action='store_true', help='Collect with existing apps/high load; preserve invalid conditions and never accept gate results')
    p.add_argument('--session-lock-inode', type=int, help='Reuse the caller-owned onscreen lock by verified inode; caller must retain/release it')
    p.add_argument('--reference', type=Path, default=Path('/tmp/pm-ref-v1.2.0'))
    p.add_argument('--native', type=Path, default=Path.home() / 'Developer/pm-target/release/powermove-native')
    p.add_argument('--corpus', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--runs', type=int, default=30)
    p.add_argument('--playback-runs', type=int, default=3)
    p.add_argument('--refresh-hz', type=float, default=60, help='Explicitly configured refresh rate; never auto-certified as either required rig')
    p.add_argument('--idle-seconds', type=float, default=2)
    p.add_argument('--settle-seconds', type=float, default=3)
    p.add_argument('--lock-wait-seconds', type=int, default=300)
    args = p.parse_args()
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt('Signal received; clean up owned app and GUI lock')
    signal.signal(signal.SIGTERM, interrupted)
    if min(args.runs, args.idle_seconds, args.settle_seconds, args.refresh_hz) <= 0 or not 0 <= args.playback_runs <= args.runs:
        p.error('Positive runs/durations/rate required; playback runs must be between zero and runs')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    if not args.output.parent.resolve().is_relative_to(Path('/private/tmp')):
        p.error('Raw output must be outside Git under /tmp')
    if args.output.is_symlink():
        p.error('Refusing a symlink for raw results')
    data = run(args)
    for row, result in data['results'].items():
        print(f'{row}: {result["status"]}')
    for blocker in data['blockers']:
        print('BLOCKED:', blocker)
    return 0 if all(r['status'] == 'PASS' for r in data['results'].values()) else 2


if __name__ == '__main__':
    sys.exit(main())
