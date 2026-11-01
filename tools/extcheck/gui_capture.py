"""Acquire the same POSIX lock as macOS lockf; never wait behind another GUI lane."""
import fcntl
import subprocess
import sys

with open('/tmp/pm-shots/gui.lock', 'a') as lock:
    try:
        fcntl.lockf(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        raise SystemExit(75)
    state = subprocess.check_output(['/tmp/pm-shots/is_locked.sh'], text=True).strip()
    if state != 'unlocked':
        raise SystemExit(76)
    # Keep this parent alive to own the lock through driver shutdown.
    raise SystemExit(subprocess.run(sys.argv[1:]).returncode)
