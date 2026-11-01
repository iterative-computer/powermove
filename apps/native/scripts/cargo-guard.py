#!/usr/bin/env python3
"""Run only this lane's Cargo process, stopping it before disk crosses 2.5 GiB."""
import os
import signal
import subprocess
import sys
from pathlib import Path

# Never allow an invocation without the user-required shared target.
os.environ["CARGO_TARGET_DIR"] = str(Path.home() / "Developer" / "pm-target")

MIN_FREE = int(2.5 * 1024**3)
def free():
    stat = os.statvfs('/')
    return stat.f_bavail * stat.f_frsize

if free() < MIN_FREE:
    sys.exit('S2 disk guard: below 2.5 GiB free; no build started')
process = subprocess.Popen(['cargo', *sys.argv[1:]], start_new_session=True)
try:
    while True:
        try:
            sys.exit(process.wait(timeout=1))
        except subprocess.TimeoutExpired:
            if free() < MIN_FREE:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                sys.exit('S2 disk guard: stopped own Cargo group below 2.5 GiB free')
except KeyboardInterrupt:
    os.killpg(process.pid, signal.SIGTERM)
    raise
