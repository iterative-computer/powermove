#!/usr/bin/env python3
"""Opt-in real-app measurement. Requires the app's PM_PERF_GATE stdin seam.
No unlock, profile discovery, source mutation, or screenshot publication.
Usage: foreground.py /tmp/corpus-project.json --hz 120 --seconds 60
Run from a shell after sourcing engine/env.sh. Build before taking the GUI lock.
"""
import argparse, json, os, signal, subprocess, time
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('project', type=Path)
p.add_argument('--hz', type=int, choices=[60,120], default=120)
p.add_argument('--seconds', type=int, default=60)
p.add_argument('--scrub', action='store_true', help='pause for external physical scrub clicks; do not use stdin seek as physical input')
a=p.parse_args()
assert 1 <= a.seconds <= 180
project=a.project.resolve()
assert project.is_relative_to(Path('/tmp').resolve()), 'use a disposable /tmp corpus copy'
root=Path(__file__).resolve().parents[5]
os.chdir(root)
subprocess.run(['swift','apps/native/spikes/s3/screen-check.swift'],check=True)
assert os.statvfs('/').f_bavail*os.statvfs('/').f_frsize >= 6*1024**3, 'STOP: less than 6 GiB free'
lock=Path('/tmp/pm-shots/gui.lock'); lock.parent.mkdir(exist_ok=True)
deadline=time.monotonic()+600
while True:
    try:
        with lock.open('x') as f: f.write('engsvc2\n')
        break
    except FileExistsError:
        print('Waiting for GUI owner:',lock.read_text().strip(),flush=True)
        if time.monotonic()>deadline: raise TimeoutError('GUI lock not acquired')
        time.sleep(30)
process=None
def terminate(signum, frame):
    raise SystemExit(128 + signum)
signal.signal(signal.SIGTERM, terminate)
try:
    # Recheck after acquisition. No on-screen batch exceeds ten minutes.
    subprocess.run(['swift','apps/native/spikes/s3/screen-check.swift'],check=True)
    prefix=f'/tmp/es2-app-{a.hz}' + ('-scrub' if a.scrub else '')
    env=os.environ.copy()
    env.update(TMPDIR='/tmp',PM_PERF_GATE='1',PM_NATIVE_VERIFY_HZ=str(a.hz),PM_ENGINE_TRACE=prefix+'.trace.json')
    env['DYLD_LIBRARY_PATH']=env['PM_FFMPEG_DIR']+'/lib'
    binary=Path.home()/'Developer/pm-target'/os.getenv('S3_PROFILE','debug')/'powermove-native'
    profile=prefix+'-profile-'+str(os.getpid())
    with open(prefix+'.stdout.log','w') as output, open(prefix+'.stderr.log','w') as errors:
        process=subprocess.Popen([str(binary),'--profile',profile,str(project)],env=env,stdin=subprocess.PIPE,stdout=output,stderr=errors,text=True,start_new_session=True)
        def send(op):
            process.stdin.write(json.dumps({'op':op})+'\n'); process.stdin.flush()
        ready=time.monotonic()+60
        while True:
            if process.poll() is not None: raise RuntimeError('App exited; inspect '+prefix+'.stderr.log')
            send('status'); time.sleep(.2)
            lines=Path(prefix+'.stdout.log').read_text().splitlines()
            rows=[json.loads(s[8:]) for s in lines if s.startswith('PM_PERF ')]
            if any(r.get('frame',{}).get('display_host_s') for r in rows if isinstance(r.get('frame'),dict)): break
            if time.monotonic()>ready: raise TimeoutError('No positive real-app presentedTime within 60 s; inspect logs')
        if not a.scrub: send('play')
        start_host=time.monotonic()
        Path(prefix+'.window.json').write_text(json.dumps({'start_host':start_host,'end_host':start_host+a.seconds}))
        print('Real app ready; '+('click/drag the timeline scrub strip now' if a.scrub else 'playback measuring')+f' for {a.seconds} s',flush=True)
        time.sleep(a.seconds)
        send('pause'); send('quit')
        process.wait(timeout=30)
    print('Trace and logs:',prefix,flush=True)
    print('Analyze only positive display timestamps. A/V uses matching audio_host values; absent audio samples remain UNVERIFIED.')
finally:
    if process and process.poll() is None:
        os.killpg(process.pid,signal.SIGTERM)
        try: process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid,signal.SIGKILL); process.wait()
    lock.unlink()
