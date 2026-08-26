#!/bin/sh

if [ "$1" = "--version" ]; then
  printf '%s\n' 'codex-cli 0.147.0-fake'
  exit 0
fi

if [ "$1" = "exec" ] && [ "$2" = "--help" ]; then
  printf '%s\n' '--ephemeral --skip-git-repo-check --ignore-rules --sandbox --output-schema --output-last-message --json --model --config --image --search --add-dir --approve-for-me --dangerously-bypass-approvals-and-sandbox'
  exit 0
fi

output_path=''
resuming=0
previous=''
for argument in "$@"; do
  if [ "$previous" = '--output-last-message' ]; then output_path="$argument"; fi
  if [ "$argument" = 'resume' ]; then resuming=1; fi
  previous="$argument"
done

if [ -n "${FAKE_CODEX_INVOCATIONS:-}" ]; then
  if [ "$resuming" -eq 1 ]; then printf '%s\n' 'resume' >> "$FAKE_CODEX_INVOCATIONS"
  else printf '%s\n' 'fresh' >> "$FAKE_CODEX_INVOCATIONS"
  fi
fi

if [ "${FAKE_CODEX_MODE:-success}" = 'stale-resume' ] && [ "$resuming" -eq 1 ]; then
  printf '%s\n' 'Error: session is unknown or no longer exists' >&2
  exit 17
fi

fixture_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
while IFS= read -r line || [ -n "$line" ]; do printf '%s\n' "$line"; done < "$fixture_directory/recorded-transcript.ndjson"

if [ "${FAKE_CODEX_MODE:-success}" = 'hang' ]; then
  if [ -n "${FAKE_CODEX_PID_FILE:-}" ]; then printf '%s\n' "$$" > "$FAKE_CODEX_PID_FILE"; fi
  trap 'exit 143' TERM INT
  while :; do sleep 1; done
fi

if [ -z "$output_path" ]; then
  printf '%s\n' 'fake codex did not receive --output-last-message' >&2
  exit 18
fi

if [ -d 'inputs' ]; then
  artifact_directory=''
  for candidate in artifacts/*; do
    if [ -d "$candidate" ]; then artifact_directory="$candidate"; break; fi
  done
  if [ -n "$artifact_directory" ]; then printf '%s\n' 'rendered output' > "$artifact_directory/deliverable.txt"; fi
  if [ -n "${FAKE_CODEX_RESULT:-}" ]; then
    printf '%s\n' "$FAKE_CODEX_RESULT" > "$output_path"
  else
    printf '%s\n' '{"summary":"done","commands":[],"artifacts":[{"path":"deliverable.txt","importToTimeline":true}],"externalActions":[],"notes":[]}' > "$output_path"
  fi
else
  printf '%s\n' '{"message":"editor done"}' > "$output_path"
fi
