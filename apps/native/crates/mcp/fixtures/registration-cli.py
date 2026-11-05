#!/usr/bin/env python3
"""Synthetic native-CLI stand-in; all paths supplied by the test's temp root."""
import json
import os
import sys
import time
from pathlib import Path

mode = os.environ['PM_REGISTRATION_TEST_MODE']
if mode == 'stall':
    time.sleep(30)
elif mode == 'flood':
    sys.stdout.write('x' * (1024 * 1024 + 1))
elif mode == 'fail':
    sys.stdout.write('x' * 131072)
    sys.stderr.write('y' * 131072 + '\nOriginal CLI error.\n')
    raise SystemExit(7)
elif mode == 'no-write':
    pass
else:
    args = sys.argv[1:]
    assert args[:2] in (['mcp', 'add'], ['mcp', 'remove'])
    config = Path(os.environ['PM_REGISTRATION_TEST_CONFIG'])
    value = json.loads(config.read_text(encoding='utf-8'))
    if args[1] == 'add':
        assert args[2:6] == ['--scope', 'user', 'powermove', '--']
        value.setdefault('mcpServers', {})['powermove'] = {'type': 'stdio', 'command': args[6], 'args': args[7:]}
    else:
        assert args[2:] == ['--scope', 'user', 'powermove']
        value.get('mcpServers', {}).pop('powermove', None)
    config.write_text(json.dumps(value), encoding='utf-8')
