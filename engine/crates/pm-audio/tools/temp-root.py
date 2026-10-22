"""Run an audio harness with an owned, leak-checked TMPDIR.

The child inherits TMPDIR/TMP/TEMP. Its temporary tree must return to the
empty baseline on success or failure; the outer guard removes that tree even
when the child panics, times out, or is interrupted. Child commands must join
any separate sessions they start before exiting. Reports stay outside it.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import shutil
import sys
import subprocess
import tempfile
import time


def snapshot(root):
    files = directories = bytes_used = 0
    entries = []
    for parent, dirs, names in os.walk(root, followlinks=False):
        directories += len(dirs)
        if len(entries) < 32:
            entries.extend(str((Path(parent) / name).relative_to(root)) for name in [*dirs, *names][:32-len(entries)])
        for name in names:
            entry = Path(parent) / name
            try:
                info = entry.lstat()
            except FileNotFoundError:
                continue
            files += 1
            bytes_used += info.st_size
    return {'files': files, 'directories': directories, 'bytes': bytes_used, 'entries': entries}


def group_exists(pgid):
    try:
        os.killpg(pgid, 0)
        return True
    except ProcessLookupError:
        return False


def wait_group(pgid, seconds):
    deadline = time.monotonic() + seconds
    while group_exists(pgid) and time.monotonic() < deadline:
        time.sleep(.05)
    return not group_exists(pgid)


def stop_child_tree(child):
    # Intermediate owners handle this signal by joining their own children,
    # including Electron's separate session, before they exit.
    deadline = time.monotonic() + 105
    if group_exists(child.pid):
        os.killpg(child.pid, signal.SIGTERM)
    try:
        child.wait(timeout=105)
    except subprocess.TimeoutExpired:
        pass
    if not wait_group(child.pid, max(0, deadline - time.monotonic())):
        os.killpg(child.pid, signal.SIGKILL)
        child.wait()
        wait_group(child.pid, 5)


def interrupted(signum, _frame):
    raise SystemExit(128 + signum)


def main():
    for signum in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, interrupted)
    if sys.argv[1:] == ['--check-current']:
        root = Path(os.environ['PM_AUDIO_TEMP_ROOT'])
        if Path(os.environ['TMPDIR']).resolve() != root.resolve():
            raise SystemExit('campaign TMPDIR no longer matches its owned root')
        state = snapshot(root)
        if state['files'] or state['directories'] or state['bytes']:
            raise SystemExit(f'temporary growth before reservation: {state}')
        print('campaign temporary directory is empty before reservation', flush=True)
        return 0
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', required=True, type=Path)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if not command:
        parser.error('a child command is required')
    report = {'started': time.time(), 'status': 'running'}
    code = 1
    root = None
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    try:
        with tempfile.TemporaryDirectory(prefix='pma-', dir='/tmp') as directory:
            root = Path(directory)
            report.update(tmpdir=str(root), before=snapshot(root))
            env = os.environ.copy()
            env.update(TMPDIR=str(root) + os.sep, TMP=str(root), TEMP=str(root),
                       MAC_CHROMIUM_TMPDIR=str(root), PM_AUDIO_TEMP_ROOT=str(root))
            args.report.write_text(json.dumps(report, indent=2) + '\n')
            child = subprocess.Popen(command, env=env, start_new_session=True)
            report['childPid'] = child.pid
            args.report.write_text(json.dumps(report, indent=2) + '\n')
            try:
                code = child.wait()
                if not wait_group(child.pid, .25):
                    report['survivingProcessGroup'] = True
                    code = code or 1
                    stop_child_tree(child)
            except BaseException:
                stop_child_tree(child)
                raise
            finally:
                report.update(childExit=child.returncode, after=snapshot(root))
                report['noGrowth'] = report['after'] == report['before']
                report['status'] = 'passed' if code == 0 and report['noGrowth'] else 'failed'
        report['removed'] = not root.exists()
    except BaseException as error:
        report.update(status='failed', error=f'{type(error).__name__}: {error}')
        code = code or 1
    finally:
        if root is not None:
            if root.exists():
                try:
                    shutil.rmtree(root)
                except OSError as error:
                    report.update(status='failed', cleanupError=str(error))
                    code = code or 1
            report['removed'] = not root.exists()
        report['finished'] = time.time()
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + '\n')
    return code if code else (0 if report['status'] == 'passed' else 1)


if __name__ == '__main__':
    raise SystemExit(main())
