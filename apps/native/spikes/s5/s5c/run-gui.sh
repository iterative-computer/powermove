#!/bin/bash
# Explicitly requested on-screen diagnostic batch; no screenshots enter Git.
set -euo pipefail
root=$(cd "$(dirname "$0")/../../../../.." && pwd)
mkdir -p /tmp/pm-shots
while ! (set -o noclobber; echo s5c > /tmp/pm-shots/gui.lock) 2>/dev/null; do
  echo 'S5c waiting for GUI lock'; sleep 30
done
trap 'rm -f /tmp/pm-shots/gui.lock' EXIT
python3 - "$root" <<'PY'
import json,os,pathlib,shutil,subprocess,sys,time
root=pathlib.Path(sys.argv[1]); source=root/'apps/native/spikes/s5/s5c'; binary=pathlib.Path.home()/'Developer/pm-target/release/pm-native-s5c-spike'
start=time.monotonic()
assert shutil.disk_usage('/').free>=6*1024**3, 'STOP under 6 GiB'
subprocess.run(['swiftc','-O',str(source/'host.swift'),'-o','/tmp/pm-s5c-host'],check=True)
stamp=time.strftime('%Y%m%dT%H%M%S')
def run(name,args,env=None,timeout=100):
    assert time.monotonic()-start<480, 'GUI batch approaching 10 minutes'
    log=pathlib.Path('/tmp')/f's5c-{stamp}-{name}.jsonl'
    with log.open('w') as f:
        p=subprocess.Popen(args,env=env,stdout=f,stderr=subprocess.STDOUT)
        deadline=min(start+590,time.monotonic()+timeout)
        try:
            while p.poll() is None:
                assert shutil.disk_usage('/').free>=6*1024**3, 'STOP under 6 GiB'
                assert time.monotonic()<deadline, 'S5c bounded GUI deadline'
                try:p.wait(timeout=1)
                except subprocess.TimeoutExpired:pass
        except BaseException:
            p.kill();p.wait();raise
    print(json.dumps({'case':name,'exit':p.returncode,'log':str(log)}),flush=True)
    if p.returncode: raise SystemExit(p.returncode)
run('dom-A',['/tmp/pm-s5c-host','A',str(source)])
config=f'/tmp/s5c-ws-{os.getpid()}'
with open(f'/tmp/s5c-{stamp}-dom-worker.jsonl','w') as f:
    worker=subprocess.Popen([str(binary),'dom'],env=dict(os.environ,S5C_CONFIG=config),stdout=f,stderr=f)
    try:
        for _ in range(100):
            if pathlib.Path(config).exists():break
            assert worker.poll() is None
            time.sleep(.05)
        run('dom-B',['/tmp/pm-s5c-host','B',str(source),config])
    finally:
        worker.terminate();worker.wait();pathlib.Path(config).unlink(missing_ok=True)
run('dom-H',['/tmp/pm-s5c-host','H',str(source),str(binary)])
if os.environ.get('S5C_FOCUS')=='1':sys.exit(0)
run('life-B',['/tmp/pm-s5c-host','life',str(source),str(binary)],timeout=100)
old=pathlib.Path.home()/'Developer/pm-target/debug/pm-native-s5-spike'
for case in ['s5b-continuation','s5b-images-scheme_pattern_png','s5b-images-scheme_noise_png','lifecycle-s5b-policy']:
    run(case,[str(old)],env=dict(os.environ,S5_CASE=case),timeout=170 if case.startswith('lifecycle') else 90)
print(json.dumps({'batch_seconds':time.monotonic()-start}),flush=True)
PY
