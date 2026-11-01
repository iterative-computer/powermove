#!/usr/bin/env python3
"""Regression: a fresh editor's first action is palette -> Save Workspace -> Esc.

Uses disposable profiles, real keyboard input and the external AX assertions.
Acquires the shared GUI lock, refuses a locked screen and quits every owned app.
No screenshots, system preference changes, production profiles or Cargo builds.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--under-gui-lock", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not args.binary.is_file() or not 1 <= args.runs <= 20:
        parser.error("a built native binary and 1–20 runs are required")
    screen = Path("/tmp/pm-shots/is_locked.sh")
    if not screen.is_file() or subprocess.check_output([str(screen)], text=True).strip() != "unlocked":
        raise SystemExit("BLOCKED: an unlocked screen and the screen-state script are required")
    if not args.under_gui_lock:
        return subprocess.call([
            "/usr/bin/lockf", "-kn", "/tmp/pm-shots/gui.lock", sys.executable,
            str(Path(__file__).resolve()), *sys.argv[1:], "--under-gui-lock",
        ])
    if shutil.disk_usage("/tmp").free < 8 * 1024**3:
        raise SystemExit("BLOCKED: at least 8 GiB free is required")
    root = Path("/tmp/pm-a11y")
    root.mkdir(exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix="startup-", dir=root))
    probe = output / "shell-ax"
    subprocess.run(["swiftc", str(Path(__file__).with_name("shell-ax.swift")), "-o", str(probe)], check=True)

    def keys(*lines):
        command = ["osascript", "-e", 'tell application "System Events"']
        for line in lines:
            command.extend(["-e", line])
        subprocess.run([*command, "-e", "end tell"], check=True, stdout=subprocess.DEVNULL, timeout=10)

    for index in range(args.runs):
        if subprocess.check_output([str(screen)], text=True).strip() != "unlocked":
            raise SystemExit("BLOCKED: screen locked during verification")
        log_path = output / f"{index}.log"
        with log_path.open("w") as log:
            app = subprocess.Popen(
                [str(args.binary.resolve()), "--profile", str(output / f"profile-{index}")],
                stdin=subprocess.PIPE, stdout=log, stderr=log, text=True,
                env=dict(os.environ, PM_PERF_GATE="1", RUST_BACKTRACE="1"),
            )
            try:
                deadline = time.monotonic() + 15
                while '"ready":true' not in log_path.read_text():
                    if app.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError(f"native startup failed; see {log_path}")
                    time.sleep(0.1)
                # Activation is allowed; never set AX focus on a control.
                keys(f'tell (first process whose unix id is {app.pid}) to set frontmost to true')
                # No frame/query delay between these keys: this exposed a
                # synchronous GPUI key-window callback during popup teardown.
                keys('keystroke "k" using command down', 'keystroke "Save Layout"', 'key code 36')
                time.sleep(0.5)
                result = subprocess.run(
                    [str(probe), str(app.pid), "--sheet", "--expect-focus", "Workspace Name"],
                    capture_output=True, text=True, timeout=15,
                )
                (output / f"{index}-ax.txt").write_text(result.stdout + result.stderr)
                if result.returncode or app.poll() is not None:
                    raise RuntimeError(f"fresh palette/sheet AX regression; see {output}")
                keys("key code 53")
                print(f"PASS {index + 1}/{args.runs}: fresh palette, sheet roles/labels/order/focus, Escape", flush=True)
            finally:
                if app.poll() is None:
                    try:
                        app.stdin.write('{"op":"quit"}\n')
                        app.stdin.flush()
                        app.wait(timeout=5)
                    except (BrokenPipeError, subprocess.TimeoutExpired):
                        app.terminate()
                        app.wait(timeout=5)
        if app.returncode:
            raise RuntimeError(f"owned native app exited {app.returncode}; see {log_path}")
    print(f"AX dumps and app logs: {output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
