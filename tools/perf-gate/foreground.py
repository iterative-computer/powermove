#!/usr/bin/env python3
"""Run the service foreground harness with the native presentation clock origin.

Python 3.9 on macOS can use a process-relative monotonic origin. Durations remain
correct, but joining that clock to NSEvent/Metal host seconds excludes all rows.
The host clock preserves monotonic deadlines and gives window markers the same
origin as the native app. All original profile, lock and timeout guards remain.
"""
from pathlib import Path
import runpy
import time
from macos import host_seconds

time.monotonic = host_seconds
runpy.run_path(str(Path(__file__).resolve().parents[2] /
                  'apps/native/crates/engine-service/tools/foreground.py'), run_name='__main__')
