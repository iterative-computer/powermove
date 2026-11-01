#!/usr/bin/env bash
set -euo pipefail
source /workspace/.tools/activate.sh
cd /workspace/powermove
# An explicit scratch destination is required; never write in the source tree.
: "${PM_INSPECTOR_VERIFY_DIR:?Set PM_INSPECTOR_VERIFY_DIR to a new scratch directory}"
python3 apps/native/crates/panel-inspector/scripts/verify.py "$PM_INSPECTOR_VERIFY_DIR"
