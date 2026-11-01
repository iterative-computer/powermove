#!/usr/bin/env python3
"""Pinned-toolchain build; only the shared target, stop own build below 4 GiB."""
import os, signal, subprocess, sys
from pathlib import Path
os.chdir(Path(__file__).resolve().parent / "isolated")
os.environ['CARGO_TARGET_DIR'] = str(Path.home() / 'Developer/pm-target')
os.environ['CARGO_PROFILE_DEV_DEBUG'] = '0'
os.environ['CARGO_PROFILE_TEST_DEBUG'] = '0'
def free():
    s = os.statvfs('/')
    return s.f_bavail * s.f_frsize
if free() < 4 * 1024**3:
    sys.exit('S3 STOP: less than 4 GiB free')
p = subprocess.Popen(['cargo', *sys.argv[1:]], start_new_session=True)
try:
    while True:
        try:
            sys.exit(p.wait(timeout=1))
        except subprocess.TimeoutExpired:
            if free() < 4 * 1024**3:
                os.killpg(p.pid, signal.SIGTERM)
                p.wait(timeout=10)
                sys.exit('S3 STOP: own build stopped below 4 GiB free')
except BaseException:
    if p.poll() is None:
        os.killpg(p.pid, signal.SIGTERM)
    raise
