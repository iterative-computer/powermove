#!/usr/bin/env python3
"""Five-minute release-app stress: full-screen Space, background, hide, activate, off-screen.
Build first. Uses a temporary profile; never touches an existing app process or profile.
"""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile
import time
from gui_guard import GuiLock, check_user

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, default=Path.home() / 'Developer/pm-target/release/powermove-native')
parser.add_argument('--seconds', type=int, default=300)
args = parser.parse_args()
if args.seconds < 300:
    parser.error('crash acceptance requires at least 300 seconds')
if not args.binary.is_file():
    parser.error('build the release app first')
root = Path(__file__).resolve().parent
artifacts = Path(os.environ.get('PTEXT_ARTIFACTS', '/tmp/pm-p120b/lifecycle'))
artifacts.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='drawable-helper-') as temp:
    helper = Path(temp) / 'lifecycle'
    subprocess.run(['swiftc', root / 'drawable-lifecycle.swift', '-o', helper], check=True)
    with GuiLock():
        check_user()
        # Check console state without requesting permission or changing it.
        state = subprocess.check_output(['ioreg', '-n', 'Root', '-d1', '-a'], text=True)
        if '<key>IOConsoleLocked</key>\n\t\t<true/>' in state:
            raise RuntimeError('UNVERIFIED: console locked')
        profile = artifacts / time.strftime('profile-%Y%m%d-%H%M%S')
        subprocess.run([helper, args.binary.resolve(), profile, str(args.seconds)], check=True)
