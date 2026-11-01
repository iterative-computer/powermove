#!/usr/bin/env python3
"""Timestamped AppKit input measurement. run.sh owns the GUI lock and timeout."""
import os
from pathlib import Path
import re
import subprocess
import time
import sys

root = Path(__file__).resolve().parents[4]
os.chdir(root)
subprocess.run(['swiftc', 'apps/native/spikes/s3/scrub.swift', '-o', '/tmp/s3-scrub'], check=True)
cached = '--cached' in sys.argv
prefix = '/tmp/s3b-input-cached' if cached else '/tmp/s3-input'
log = Path(prefix + '.log')
with log.open('w') as output:
    process = subprocess.Popen(['zsh', 'apps/native/spikes/s3/run.sh', '--seconds=12', *[a for a in sys.argv[1:] if a != '--cached'], '--report=' + prefix + '.json'], stdout=output, stderr=output)
    try:
        # Acquisition retries every 30 s inside run.sh; do not steal its lock.
        while True:
            match = re.search(r'S3 window_id=(\d+)', log.read_text())
            if match:
                break
            if process.poll() is not None:
                raise RuntimeError(log.read_text())
            time.sleep(.2)
        pid = subprocess.check_output(['pgrep', '-x', 'pm-native-s3-spike'], text=True).strip()
        assert pid.isdigit(), 'expected exactly one S3 app'
        subprocess.run(['osascript', '-e', f'tell application "System Events" to set frontmost of (first process whose unix id is {pid}) to true'], check=True)
        time.sleep(2)
        subprocess.run(['/tmp/s3-scrub', pid, match.group(1), *(['--cached'] if cached else [])], check=True)
        process.wait(timeout=45)
        if process.returncode:
            raise RuntimeError(log.read_text())
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
