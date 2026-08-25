#!/bin/sh
set -eu

PROBE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
MODE=${1:-file}
ELECTRON_BIN="$PROBE_DIR/node_modules/.bin/electron"
ELECTRON_APP="$PROBE_DIR/node_modules/electron/dist/Electron.app"

case "$MODE" in
  file|app|interactive)
    exec "$ELECTRON_BIN" "$PROBE_DIR" "--mode=$MODE"
    ;;
  finder)
    RESULT="$PROBE_DIR/results-finder.json"
    rm -f "$RESULT"
    open -n "$ELECTRON_APP" --args "$PROBE_DIR" --mode=finder
    elapsed=0
    while [ ! -f "$RESULT" ] && [ "$elapsed" -lt 105 ]; do
      sleep 1
      elapsed=$((elapsed + 1))
    done
    if [ ! -f "$RESULT" ]; then
      echo "Timed out waiting for $RESULT" >&2
      exit 1
    fi
    cat "$RESULT"
    ;;
  *)
    echo "Usage: $0 [file|app|finder|interactive]" >&2
    exit 2
    ;;
esac
