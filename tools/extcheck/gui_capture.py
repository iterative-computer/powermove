"""Wait for the shared GUI lease, preserving other lanes' lock files."""
from contextlib import contextmanager
from pathlib import Path
import fcntl
import os
import signal
import subprocess
import sys
import time
from resources import disk_ready

def gui_unavailable():
    return (Path('/tmp/pm-shots/PAUSE_LOCAL').exists()
            or Path('/tmp/pm-shots/USER_ACTIVE').exists())

@contextmanager
def gui_slot():
    with open('/tmp/pm-shots/gui.lock', 'a') as lock:
        while True:
            while gui_unavailable():
                time.sleep(300)
            try:
                fcntl.lockf(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                time.sleep(60)
                continue
            state = subprocess.check_output(['/tmp/pm-shots/is_locked.sh'], text=True).strip()
            if state == 'unlocked' and not gui_unavailable() and disk_ready():
                break
            fcntl.lockf(lock, fcntl.LOCK_UN)
            time.sleep(60)
        try:
            yield
        finally:
            fcntl.lockf(lock, fcntl.LOCK_UN)

def stop_owned_process(child):
    # Playwright's Electron child has its own group. Its signal handler forces
    # that tracked child closed when asked a second time; don't SIGKILL the
    # driver before allowing that cleanup path to run.
    for _ in range(2):
        try:
            os.killpg(child.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            child.wait(timeout=1)
            return
        except subprocess.TimeoutExpired:
            pass
    try:
        os.killpg(child.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    child.wait()

def guarded_run(command):
    if gui_unavailable() or not disk_ready():
        return 77
    child = subprocess.Popen(command, start_new_session=True)
    try:
        while True:
            if gui_unavailable() or not disk_ready():
                stop_owned_process(child)
                return 77
            try:
                return child.wait(timeout=.25)
            except subprocess.TimeoutExpired:
                pass
    finally:
        if child.poll() is None:
            stop_owned_process(child)

if __name__ == '__main__':
    if not sys.argv[1:]:raise SystemExit('capture command required')
    with gui_slot():
        raise SystemExit(guarded_run(sys.argv[1:]))
