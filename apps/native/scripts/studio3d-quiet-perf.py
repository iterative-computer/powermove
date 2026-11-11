#!/usr/bin/env python3
"""UNVERIFIED measurement driver; validate before using it for acceptance.
Run only in a quiet window, under nice -n 10 flock /tmp/pm-build.lock.
Records real presentedTime and native input timestamps; never synthesizes fps.
The operator navigates with real pointer/key events during the input phase.
"""
import argparse, hashlib, json, os, pathlib, queue, re, shutil, subprocess, threading, time


def allowed():
    if pathlib.Path('/tmp/pm-shots/USER_ACTIVE').exists():
        raise RuntimeError('USER_ACTIVE: timing is paused')
    info = subprocess.check_output(['diskutil', 'info', '/'], text=True)
    match = re.search(r'Container Free Space:.*?\((\d+) Bytes\)', info)
    if not match or int(match[1]) < 5_000_000_000:
        raise RuntimeError('Need at least 5 GB before a measurement')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', required=True, type=pathlib.Path)
    parser.add_argument('--project', required=True, type=pathlib.Path)
    parser.add_argument('--out', required=True, type=pathlib.Path)
    parser.add_argument('--seconds', type=int, default=60)
    parser.add_argument('--input-seconds', type=int, default=30)
    args = parser.parse_args()
    allowed()
    out = args.out.resolve()
    if not str(out).startswith('/private/tmp/pm-shots/') and not str(out).startswith('/tmp/pm-shots/'):
        raise RuntimeError('Evidence output must be under /tmp/pm-shots')
    out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()):
        raise RuntimeError('Use a fresh empty output directory')
    project = out / 'corpus.pmv'
    shutil.copyfile(args.project, project)
    trace_path = out / 'present.jsonl'
    env = dict(os.environ, PM_PERF_GATE='1', PM_NATIVE_STUDIO_TIER='live',
               PM_ENGINE_TRACE=str(trace_path), STUDIO_TIMINGS='1')
    app = args.app.resolve()
    (out / 'binary-sha256.txt').write_text(hashlib.sha256(app.read_bytes()).hexdigest() + '\n')
    replies = queue.Queue()
    stdout = (out / 'stdout.log').open('w')
    stderr = (out / 'stderr.log').open('w')
    process = subprocess.Popen([str(app), '--profile', str(out / 'profile'), str(project)],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr,
                               text=True, env=env)

    def read():
        for line in process.stdout:
            stdout.write(line); stdout.flush()
            if line.startswith('PM_PERF '):
                replies.put(json.loads(line[len('PM_PERF '):]))
    threading.Thread(target=read, daemon=True).start()

    def command(op):
        process.stdin.write(json.dumps({'op': op}) + '\n'); process.stdin.flush()
        response = replies.get(timeout=30)
        if response.get('error'):
            raise RuntimeError(response['error'])
        return response

    def wait(seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if pathlib.Path('/tmp/pm-shots/USER_ACTIVE').exists():
                raise RuntimeError('USER_ACTIVE: run interrupted')
            if process.poll() is not None:
                raise RuntimeError('App exited during measurement')
            time.sleep(min(.25, max(0, deadline - time.monotonic())))

    try:
        replies.get(timeout=30)  # controller-ready, not a presented frame
        deadline = time.monotonic() + 60
        while True:
            status = command('status')
            if (status.get('frame') or {}).get('display_host_s'):
                break
            if time.monotonic() > deadline:
                raise RuntimeError('No presented first frame')
            wait(.2)
        command('play'); wait(8)  # warm shaders/temporal history
        start = command('status')['accepted_host_s']
        wait(args.seconds)
        end = command('pause')['accepted_host_s']
        wait(2)
        input_start = command('status')['accepted_host_s']
        print('INPUT PHASE: navigate the viewer using real pointer/key events now. '
              'Keep the shot paused; collect at least 100 distinct presented inputs.', flush=True)
        wait(args.input_seconds)
        input_end = command('status')['accepted_host_s']
        command('quit'); process.wait(timeout=20)
    finally:
        if process.poll() is None:
            process.terminate()
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired: process.kill(); process.wait()
        stdout.close(); stderr.close()
    rows = [json.loads(line) for line in trace_path.read_text().splitlines() if line.strip()]
    rows = [r for r in rows if r.get('gpu_ok') and not r.get('stale') and r.get('display', 0) > 0]
    displayed = sorted(set(r['display'] for r in rows if start <= r['display'] <= end))
    if len(displayed) < 2:
        raise RuntimeError('Insufficient presented frames; use an animated 60-fps composition')
    gaps = [b-a for a, b in zip(displayed, displayed[1:])]
    gaps += [max(0, displayed[0]-start), max(0, end-displayed[-1])]
    inputs = {}
    for row in rows:
        event = row.get('input')
        if event and input_start <= event <= input_end and row['display'] >= event:
            inputs[event] = min(inputs.get(event, float('inf')), row['display']-event)
    latency = sorted(inputs.values())
    p95 = latency[max(0, (len(latency)*95+99)//100-1)]*1000 if latency else None
    result = {'presented_frames': len(displayed), 'presented_fps': (len(displayed)-1)/(displayed[-1]-displayed[0]),
              'max_present_gap_ms': max(gaps)*1000, 'stalls_at_least_100ms': sum(g >= .1 for g in gaps),
              'distinct_presented_native_inputs': len(latency), 'input_to_pixel_p95_ms': p95,
              'input_sample_sufficient': len(latency) >= 100,
              'live_gate': (len(displayed)-1)/(displayed[-1]-displayed[0]) >= 60 and max(gaps) < .1,
              'input_gate': len(latency) >= 100 and p95 <= 50}
    (out / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
