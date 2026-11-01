#!/bin/bash
# Run only on an unlocked Mac. Every input/output lives in a fresh temp directory.
set -euo pipefail
mode="${1:-close}"
case "$mode" in close|recovery) ;; *) echo 'Usage: check-lifecycle-ui.sh [close|recovery]' >&2; exit 2;; esac
binary="${CARGO_TARGET_DIR:-$HOME/Developer/pm-target}/debug/powermove-native"
[ -x "$binary" ] || { echo 'Build pm-native-app with the pinned toolchain first.' >&2; exit 1; }
root=$(mktemp -d /tmp/pm-native-lifecycle.XXXXXX)
python3 - "$binary" "$root" "$mode" <<'PY'
import json, pathlib, subprocess, sys
binary, root, mode = sys.argv[1:]
root = pathlib.Path(root)
project = root / 'project.json'
project.write_text(json.dumps({'id':'ui-check','name':'Disposable UI check','compId':'main','w':320,'h':180,'fps':30,'dur':2,'bg':'#000000','layers':[{'id':'base-layer','type':'shape','name':'Base layer','from':0,'dur':2,'on':True,'p':{'opacity':{'v':100,'kf':[],'expr':None}}}],'assets':{},'comps':{}}))
if mode == 'recovery':
    child = subprocess.Popen([binary, '--powermove-mcp', '--profile', str(root/'profile'), str(project)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    requests = [('initialize',{}), ('tools/call', {'name':'start_session','arguments':{'prompt':'Disposable recovery check'}}), ('tools/call',{'name':'apply_commands','arguments':{'commands':[{'type':'add_layer','id':'recovered-layer','name':'Recovered work','layerType':'solid'}]}}), ('tools/call',{'name':'finish_session','arguments':{'commit':True}})]
    for i, (method, params) in enumerate(requests):
        child.stdin.write(json.dumps({'jsonrpc':'2.0','id':i,'method':method,'params':params})+'\n'); child.stdin.flush()
        response = json.loads(child.stdout.readline())
        if 'error' in response or response.get('result',{}).get('isError'):
            raise RuntimeError('Recovery setup failed')
    child.stdin.close()
    if child.wait() != 0: raise RuntimeError('Recovery host failed')
PY
printf 'Disposable check directory: %s\n' "$root"
if [ "$mode" = recovery ]; then
  echo 'UNVERIFIED: Check native Recover / Open as Copy / Discard sheet. Repeat this script for each choice.'
  echo 'Recover/copy: the recovered layer appears. Discard: original keeps only the base layer; journals contain a discarded archive.'
else
  echo 'UNVERIFIED: Select the base layer and change opacity; close via red traffic light, Command-W and Command-Q in separate runs.'
  echo 'Check Save / Don’t Save / Cancel. Escape cancels. Command-D chooses Don’t Save.'
  echo 'Check edited dot, saved reopen, unchanged original after Don’t Save, and refusal/progress note during a save.'
  echo 'With the editor closed, attach --powermove-mcp --profile to this profile; native Quit must still work.'
fi
exec "$binary" --profile "$root/profile" "$root/project.json"
