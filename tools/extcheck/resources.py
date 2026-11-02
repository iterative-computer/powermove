"""The lane's shared APFS container floor, measured by diskutil (decimal GB)."""
import re
import subprocess
import time

MIN_FREE_BYTES = 5_000_000_000
_sample = (0.0, 0)


def container_free_bytes():
    global _sample
    now = time.monotonic()
    if now - _sample[0] < .5:
        return _sample[1]
    output = subprocess.check_output(['diskutil', 'info', '/'], text=True)
    match = re.search(r'Container Free Space:.*?\((\d+) Bytes\)', output)
    if not match:
        raise RuntimeError('APFS container free space is unavailable')
    _sample = (now, int(match[1]))
    return _sample[1]


def disk_ready():
    return container_free_bytes() >= MIN_FREE_BYTES
