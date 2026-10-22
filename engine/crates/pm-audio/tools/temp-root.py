"""Run an audio harness with an owned, leak-checked TMPDIR.

The child inherits TMPDIR/TMP/TEMP. Its temporary tree must return to the
empty baseline on success or failure; the outer guard removes that tree even
when the child panics, times out, or is interrupted. Reports stay outside it.
"""
import argparse
import json
import os
from pathlib import Path
import signal
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


def interrupted(signum, _frame):
    raise SystemExit(128 + signum)


def main():
    for signum in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, interrupted)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', required=True, type=Path)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if not command:
        parser.error('a child command is required')
    report = {'started': time.time(), 'status': 'running'}
    code = 1
    try:
        with tempfile.TemporaryDirectory(prefix='pm-audio-campaign-') as directory:
            root = Path(directory)
            report.update(tmpdir=str(root), before=snapshot(root))
            env = os.environ.copy()
            env.update(TMPDIR=str(root) + os.sep, TMP=str(root), TEMP=str(root))
            child = subprocess.Popen(command, env=env, start_new_session=True)
            try:
                code = child.wait()
            except BaseException:
                # This process group contains only this harness's child tree.
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait()
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
        report['finished'] = time.time()
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + '\n')
    return code if code else (0 if report['status'] == 'passed' else 1)


if __name__ == '__main__':
    raise SystemExit(main())
