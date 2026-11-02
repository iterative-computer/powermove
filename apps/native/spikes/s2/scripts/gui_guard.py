"""Shared GUI lease: supports lockf users and the earlier exclusive-create marker."""
import errno
import fcntl
import os
from pathlib import Path
import subprocess
import time

GUI_LOCK = Path('/tmp/pm-shots/gui.lock')
USER_ACTIVE = GUI_LOCK.with_name('USER_ACTIVE')

def check_user():
    if USER_ACTIVE.exists():
        raise RuntimeError('STOP: USER_ACTIVE; no on-screen work')

class GuiLock:
    def __enter__(self):
        GUI_LOCK.parent.mkdir(parents=True, exist_ok=True)
        self.created = False
        while True:
            check_user()
            try:
                self.fd = os.open(GUI_LOCK, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
                self.created = True
            except FileExistsError:
                self.fd = os.open(GUI_LOCK, os.O_RDWR)
            try:
                fcntl.lockf(self.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                if not GUI_LOCK.exists() or os.stat(GUI_LOCK).st_ino != os.fstat(self.fd).st_ino:
                    raise BlockingIOError(errno.EAGAIN, 'GUI lease changed while waiting')
                # Nonempty files are exclusive-create leases from older drivers.
                # An empty pre-existing lockf file is reusable without removing it.
                if not self.created and os.fstat(self.fd).st_size:
                    raise BlockingIOError(errno.EAGAIN, 'GUI marker held')
                break
            except BlockingIOError:
                os.close(self.fd)
                self.created = False
                print('GUI lock held; waiting', flush=True)
                time.sleep(5)
        try:
            check_user()
        except BaseException:
            self.__exit__()
            raise
        return self

    def __exit__(self, *_):
        if self.created and GUI_LOCK.exists() and os.stat(GUI_LOCK).st_ino == os.fstat(self.fd).st_ino:
            GUI_LOCK.unlink()
        os.close(self.fd)

def quiet_state(max_load=3.):
    """Check actual Cargo/rustc executable names, never command-line substrings."""
    check_user()
    load = os.getloadavg()
    commands = subprocess.check_output(['ps', '-A', '-o', 'comm='], text=True)
    builds = [s.strip() for s in commands.splitlines() if Path(s.strip()).name in {'cargo', 'rustc'}]
    return {'load': load, 'builds': builds, 'quiet': load[0] <= max_load and not builds}

def wait_quiet():
    while True:
        state = quiet_state()
        if state['quiet']:
            return state
        print(f"Waiting for quiet machine: load={state['load'][0]:.2f}, builds={len(state['builds'])}", flush=True)
        time.sleep(10)
