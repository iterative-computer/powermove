#!/usr/bin/env python3
"""Run the prebuilt paired extension gate under the shared Electron lock."""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time


def stop_profiles(temporary):
    """Electron may create its own process group. Stop only this run's profiles."""
    rows = subprocess.check_output(['ps', '-axo', 'pid=,ppid=,command='], text=True)
    processes = {}
    owned = set()
    for row in rows.splitlines():
        parts = row.strip().split(None, 2)
        if len(parts) != 3:
            continue
        pid, parent = map(int, parts[:2])
        processes[pid] = parent
        if '--type=' in parts[2] and '--user-data-dir=' + str(temporary) + '/' in parts[2]:
            owned.add(pid)
    while True:
        children = {pid for pid, parent in processes.items() if parent in owned}
        if children <= owned:
            break
        owned.update(children)
    for sig in [signal.SIGTERM, signal.SIGKILL]:
        for pid in owned:
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                pass
        if owned and sig == signal.SIGTERM:
            time.sleep(.5)
    return len(owned)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    desktop = Path(__file__).resolve().parents[1]
    repo = desktop.parents[1]
    output = args.out.resolve()
    if output == repo or repo in output.parents:
        parser.error('Evidence must be outside this public repository.')
    output.mkdir(parents=True, exist_ok=True)
    required = [desktop / 'out/main/index.js', desktop / 'out/renderer/engine/pm_wasm.wasm', desktop / 'native/pm_node.node']
    if not all(p.is_file() for p in required):
        parser.error('Build the native addons, wasm, and app before running this prebuilt gate.')
    protocol = re.search(r'PROTOCOL_BUILD = "([^"]+)"', (desktop / 'src/shared/engine-protocol/index.ts').read_text()).group(1)
    actual = subprocess.check_output(['node', '-e', 'process.stdout.write(require(process.argv[1]).buildId())', str(required[-1])], cwd=desktop, text=True)
    if actual != protocol:
        parser.error(f'Rebuild the native addon: {actual} differs from {protocol}.')
    if shutil.disk_usage(output if output.exists() else output.parent).free < 8 * 1024**3:
        parser.error('Less than 8 GiB available for the run.')
    run = output / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    run.mkdir(parents=True)
    temporary = Path(tempfile.mkdtemp(prefix='tmp-', dir=run))
    env = {**os.environ, 'PM_EXTENSION_PERF_OUT': str(run), 'TMPDIR': str(temporary) + '/'}
    lock = Path.home() / 'Library/Caches/powermove-engine/bin/pm-electron-lock'
    command = [str(lock), 'nice', '-n', '15', 'bun', 'run', 'test:e2e', '--', '--config', 'playwright.extension-perf.config.ts', '--workers=1']
    workers = []
    process = None
    started = time.monotonic()
    status = {'loadBefore': os.getloadavg(), 'protocol': protocol}
    print(f'Evidence: {run}', flush=True)
    try:
        with (run / 'run.log').open('w') as log:
            process = subprocess.Popen(command, cwd=desktop, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            while process.poll() is None:
                owns_lock = any(p.exists() and p.read_text().strip() == str(process.pid) for p in Path('/tmp').glob('powermove-electron.lock*/pid'))
                if owns_lock and not workers:
                    for _ in range(2):
                        workers.append(subprocess.Popen([sys.executable, '-c', 'import hashlib; x=b"compatibility"*65536\nwhile True: x=hashlib.sha256(x).digest()*2048'], start_new_session=True))
                    print('Electron acquired; two SHA256 load workers started.', flush=True)
                if time.monotonic() - started > 1200:
                    raise TimeoutError('Paired extension benchmark exceeded 20 minutes.')
                time.sleep(.1)
            status['exitCode'] = process.returncode
    except (KeyboardInterrupt, Exception) as error:
        status['error'] = str(error)
        status['exitCode'] = 1
    finally:
        for worker in workers:
            if worker.poll() is None:
                worker.terminate()
        for worker in workers:
            try:
                worker.wait(timeout=3)
            except subprocess.TimeoutExpired:
                worker.kill()
                worker.wait()
        if process:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        status['profileProcessesStopped'] = stop_profiles(temporary)
        shutil.rmtree(temporary)
        status.update(loadAfter=os.getloadavg(), seconds=round(time.monotonic() - started, 1), temporaryRootRemoved=not temporary.exists(), loadWorkersStopped=all(w.poll() is not None for w in workers))
        (run / 'status.json').write_text(json.dumps(status, indent=2) + '\n')
        print(json.dumps(status), flush=True)
    return status['exitCode']


def interrupted(*_):
    raise KeyboardInterrupt('termination requested')


if __name__ == '__main__':
    signal.signal(signal.SIGTERM, interrupted)
    sys.exit(main())
