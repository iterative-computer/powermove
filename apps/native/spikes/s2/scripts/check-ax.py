#!/usr/bin/env python3
"""Screen-free S2 contracts/build/AX check; --exercise also tests native actions.

--no-build uses the existing spike binary; --pid checks an already running spike.
Build metadata is isolated in /tmp so parallel lanes' lockfiles stay untouched.
All Rust output goes to the one authorized shared target; logs stay outside Git.
"""
import argparse
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--no-build', action='store_true')
parser.add_argument('--exercise', action='store_true')
parser.add_argument('--pid', type=int)
args = parser.parse_args()
repo = Path(__file__).resolve().parents[5]
native = repo / 'apps/native'
spike = native / 'spikes/s2'
target = Path.home() / 'Developer/pm-target'
os.environ['CARGO_TARGET_DIR'] = str(target)
os.chdir(native)  # Select the pinned toolchain, even with a /tmp manifest.
MIN_FREE = 4 * 1024**3

def guarded(command):
    def free():
        stat = os.statvfs('/')
        return stat.f_bavail * stat.f_frsize
    if free() < MIN_FREE:
        raise SystemExit('S2b stopped: less than 4 GiB free; no build started')
    process = subprocess.Popen(command, start_new_session=True)
    try:
        while True:
            try:
                code = process.wait(timeout=1)
                if code:
                    raise SystemExit(code)
                return
            except subprocess.TimeoutExpired:
                if free() < MIN_FREE:
                    raise SystemExit('S2b stopped own build: less than 4 GiB free')
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()

subprocess.run(['df', '-h', '/'], check=True)
subprocess.run(['rustc', '--version'], check=True)
work = target / 's2b'
work.mkdir(exist_ok=True)
harness = work / 'contracts.rs'
harness.write_text('\n'.join(
    '#[path="' + str(spike / 'src' / (name + '.rs')) + '"] mod ' + name + ';'
    for name in ['design', 'contract']))
guarded(['rustc', '--edition=2024', '--test', str(harness), '-o', str(work / 'contracts')])
subprocess.run([work / 'contracts'], check=True)
guarded(['rustc', '--edition=2024', '--test', str(native / 'vendor/gpui-ce/crates/gpui_macos/src/composition_accessibility_order.rs'), '-o', str(work / 'ax-order-tests')])
subprocess.run([work / 'ax-order-tests'], check=True)
if not args.no_build and not args.pid:
    # Stable /tmp metadata path allows reuse; source and dependencies are exact
    # checkout paths. Cargo may adjust only this copied lock, never the shared one.
    metadata = Path(tempfile.gettempdir()) / 's2b-build'
    metadata.mkdir(exist_ok=True)
    manifest = (spike / 'Cargo.toml').read_text()
    manifest = re.sub(r'path = "(../../vendor/[^"]+)"',
                      lambda m: 'path = "' + str((spike / m[1]).resolve()) + '"', manifest)
    manifest += '\n[workspace]\n[profile.dev]\ndebug = 0\nincremental = false\n[profile.test]\ndebug = 0\nincremental = false\n'
    (metadata / 'Cargo.toml').write_text(manifest)
    source = metadata / 'src'
    if source.is_symlink() and source.resolve() != spike / 'src':
        source.unlink()
    if not source.exists():
        source.symlink_to(spike / 'src', target_is_directory=True)
    elif source.resolve() != spike / 'src':
        raise SystemExit('S2b temporary source path is not this checkout; refusing to build')
    (metadata / 'Cargo.lock').write_bytes((native / 'Cargo.lock').read_bytes())
    guarded(['cargo', 'build', '--offline', '--manifest-path', str(metadata / 'Cargo.toml'), '-j', '3'])
# Compile a small AppKit-only executable from the exact vendored sources. It
# never orders its NSWindow on screen and checks removal and adapter teardown.
probe = Path(tempfile.gettempdir()) / 's2b-bridge-probe'
probe.mkdir(exist_ok=True)
(probe / 'Cargo.toml').write_text('[package]\nname="s2b-bridge-probe"\nversion="0.1.0"\nedition="2024"\n[workspace]\n[[bin]]\nname="s2b-bridge-probe"\npath="' + str(spike / 'scripts/ax-bridge-probe.rs') + '"\n[dependencies]\naccesskit="=0.24.1"\naccesskit_macos="=0.26.3"\nobjc2="0.6"\nobjc2-app-kit="0.3"\nobjc2-foundation="0.3"\ndispatch2="0.3.1"\n[profile.dev]\ndebug=0\nincremental=false\n')
(probe / 'Cargo.lock').write_bytes((native / 'Cargo.lock').read_bytes())
guarded(['cargo', 'run', '--offline', '--manifest-path', str(probe / 'Cargo.toml'), '-j', '3'])
guarded(['swiftc', str(spike / 'scripts/ax-walk.swift'), '-o', str(work / 'ax-walk')])
process = None
with tempfile.TemporaryFile(mode='w+') as log:
    try:
        if args.pid:
            pid = args.pid
        else:
            process = subprocess.Popen([target / 'debug/pm-native-s2-spike'], stdout=log, stderr=log, env={**os.environ, 'S2_AX_DIAGNOSTICS': '1'})
            pid = process.pid
        result = subprocess.run([str(work / 'ax-walk'), str(pid)] + (['--exercise'] if args.exercise else []))
        if result.returncode == 2:
            raise SystemExit(2)  # Preserve the walker's permission/usage BLOCKED result.
        if process and process.poll() is not None:
            log.seek(0)
            print(log.read())
            raise SystemExit('S2b spike exited before AX check completed')
        if process:
            log.seek(0)
            diagnostics = [line for line in log.read().splitlines() if line.startswith(('S2_CHROME ', 'S2_AX_ORDER '))]
            print('\n'.join(diagnostics))
            expected = 'NSAppearanceNameAqua' if os.environ.get('S2_APPEARANCE') == 'light' else 'NSAppearanceNameDarkAqua'
            if not any('window=' + expected in line and 'menu=' + expected in line for line in diagnostics):
                raise SystemExit('FAIL S2 effective window/menu appearance diagnostics')
            print('PASS effective window/menu appearance agrees with the spike override')
        raise SystemExit(result.returncode)
    finally:
        if process and process.poll() is None:
            process.terminate()
            try:
                process.wait(5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
