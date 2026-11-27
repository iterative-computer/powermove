#!/usr/bin/env python3
"""Alternate two fixed private projects in one native app process.
Run under nice -n 10 flock /tmp/pm-build.lock; records RSS and Metal separately.
"""
import argparse, importlib.util, json, os, pathlib, queue, shutil, subprocess, threading, time


def slope(values):
    n = len(values)
    center = (n-1)/2
    return sum((i-center)*v for i, v in enumerate(values))/sum((i-center)**2 for i in range(n))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--app', required=True, type=pathlib.Path)
    p.add_argument('--a', required=True, type=pathlib.Path)
    p.add_argument('--b', required=True, type=pathlib.Path)
    p.add_argument('--out', required=True, type=pathlib.Path)
    p.add_argument('--library-dir', type=pathlib.Path)
    p.add_argument('--cycles', type=int, default=10)
    args = p.parse_args()
    if args.cycles < 10:
        p.error('At least ten measured A/B cycles are required')
    spec = importlib.util.spec_from_file_location('quiet', pathlib.Path(__file__).with_name('studio3d-quiet-perf.py'))
    quiet = importlib.util.module_from_spec(spec); spec.loader.exec_module(quiet)
    quiet.allowed()
    out = quiet.evidence_directory(args.out)
    out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()):
        raise RuntimeError('Use a new empty evidence directory')
    copies = [out/'a.pmv', out/'b.pmv']
    for source, target in zip([args.a,args.b],copies):
        shutil.copyfile(source,target)
    env = dict(os.environ, PM_PERF_GATE='1', PM_NATIVE_STUDIO_TIER='live')
    if args.library_dir:
        env['DYLD_LIBRARY_PATH'] = str(args.library_dir.resolve())
    stdout = (out/'stdout.log').open('w'); stderr = (out/'stderr.log').open('w')
    process = subprocess.Popen([str(args.app.resolve()), '--profile', str(out/'profile'), str(copies[0])], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, text=True, env=env)
    replies = queue.Queue()
    def read():
        for line in process.stdout:
            stdout.write(line); stdout.flush()
            if line.startswith('PM_PERF '): replies.put(json.loads(line[8:]))
    reader = threading.Thread(target=read,daemon=True); reader.start()
    def command(op, **fields):
        process.stdin.write(json.dumps(dict(op=op,**fields))+'\n'); process.stdin.flush()
        result = replies.get(timeout=30)
        if result.get('error'): raise RuntimeError(result['error'])
        return result
    def settle(path):
        quiet.allowed()
        old = command('status').get('viewer_id')
        request = command('open',path=str(path))
        deadline = time.monotonic()+60
        while True:
            status = command('status'); frame = status.get('frame') or {}
            if not status.get('opening') and status.get('viewer_id') != old and (frame.get('display_host_s') or 0) > request['accepted_host_s'] and frame.get('gpu_ok'):
                break
            if time.monotonic() > deadline: raise RuntimeError('Project did not present: '+json.dumps(status))
            if pathlib.Path('/tmp/pm-shots/USER_ACTIVE').exists(): raise RuntimeError('USER_ACTIVE')
            time.sleep(.1)
        time.sleep(1.5)
        return command('status')
    samples = []
    try:
        replies.get(timeout=30)
        command('focus')
        # Eight opens fill bounded frame/pipeline caches before measurement.
        for _ in range(4):
            for path in copies[::-1]: settle(path)
        for cycle in range(args.cycles):
            for project, path in [('b',copies[1]),('a',copies[0])]:
                status = settle(path)
                rss = int(subprocess.check_output(['ps','-o','rss=','-p',str(process.pid)],text=True))*1024
                row = dict(cycle=cycle, project=project, rss_bytes=rss, metal_allocated_bytes=status['metal_allocated_bytes'], viewer_id=status['viewer_id'], output=status['main_output'])
                samples.append(row)
                (out/'samples.json').write_text(json.dumps(samples,indent=2)+'\n')
        process.stdin.write('{"op":"quit"}\n'); process.stdin.flush(); process.wait(timeout=20)
    finally:
        if process.poll() is None:
            process.terminate()
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired: process.kill(); process.wait()
        reader.join(timeout=5); stdout.close(); stderr.close()
    result = dict(cycles=args.cycles, warmup_cycles=4, units='bytes; RSS and Metal overlap on unified memory and must not be added', projects={})
    for project in ['a','b']:
        rows = [r for r in samples if r['project']==project]
        result['projects'][project] = {metric: dict(first=rows[0][metric],last=rows[-1][metric],min=min(r[metric] for r in rows),max=max(r[metric] for r in rows),slope_per_cycle=slope([r[metric] for r in rows])) for metric in ['rss_bytes','metal_allocated_bytes']}
    (out/'summary.json').write_text(json.dumps(result,indent=2)+'\n'); print(json.dumps(result,indent=2))


if __name__ == '__main__': main()
