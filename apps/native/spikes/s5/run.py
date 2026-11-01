#!/usr/bin/env python3
"""Run one host at a time; private logs remain in /tmp, never in Git."""
import json, os, pathlib, subprocess, sys, time
binary = pathlib.Path.home() / 'Developer/pm-target/debug/pm-native-s5-spike'
failed=False
for case in sys.argv[1:] or ['http', 'scheme', 'prompt', 'app']:
    env = dict(os.environ, S5_CASE=case)
    path = pathlib.Path('/tmp') / ('s5-' + case + '-' + time.strftime('%Y%m%dT%H%M%S') + '-' + str(os.getpid()) + '.jsonl')
    with path.open('w') as log:
        try:
            result = subprocess.run([str(binary)], env=env, stdout=log, stderr=subprocess.STDOUT, timeout=175 if case.startswith('lifecycle-') else 35)
            failed |= result.returncode != 0
            print(json.dumps({'case':case,'exit':result.returncode,'log':str(path)}),flush=True)
        except subprocess.TimeoutExpired:
            failed=True
            print(json.dumps({'case':case,'failure':'external watchdog expired','log':str(path)}),flush=True)

sys.exit(1 if failed else 0)
