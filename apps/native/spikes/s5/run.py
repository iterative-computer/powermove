#!/usr/bin/env python3
"""Run one host at a time; private logs remain in /tmp, never in Git."""
import json, os, pathlib, subprocess, sys
binary = pathlib.Path.home() / 'Developer/pm-target/debug/pm-native-s5-spike'
for case in sys.argv[1:] or ['http', 'scheme', 'prompt', 'app']:
    env = dict(os.environ, S5_CASE=case)
    path = pathlib.Path('/tmp') / ('s5-' + case + '.jsonl')
    with path.open('w') as log:
        try:
            result = subprocess.run([str(binary)], env=env, stdout=log, stderr=subprocess.STDOUT, timeout=35)
            print(json.dumps({'case':case,'exit':result.returncode,'log':str(path)}),flush=True)
        except subprocess.TimeoutExpired:
            print(json.dumps({'case':case,'failure':'35-second external watchdog expired','log':str(path)}),flush=True)
