#!/bin/sh
# Build only. Signing and notarization are separate explicit operator commands.
set -eu
exec python3 "$(dirname "$0")/package.py" dmg "$@"
