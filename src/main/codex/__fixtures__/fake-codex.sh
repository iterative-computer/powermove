#!/bin/sh

if [ "$1" = "--version" ]; then
  printf '%s\n' 'codex-cli 0.147.0-fake'
  exit 0
fi

if [ "$1" = "exec" ] && [ "$2" = "--help" ]; then
  printf '%s\n' '--ephemeral --ignore-user-config --skip-git-repo-check --ignore-rules --sandbox --output-schema --output-last-message --json --model --config --image --search --disable --add-dir --approve-for-me --dangerously-bypass-approvals-and-sandbox'
  exit 0
fi

output_path=''
resuming=0
ignore_user_config=0
previous=''
for argument in "$@"; do
  if [ "$previous" = '--output-last-message' ]; then output_path="$argument"; fi
  if [ "$argument" = 'resume' ]; then resuming=1; fi
  if [ "$argument" = '--ignore-user-config' ]; then ignore_user_config=1; fi
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

if [ "${FAKE_CODEX_MODE:-success}" = 'mcp-fallback' ] && [ "$ignore_user_config" -eq 0 ]; then
  printf '%s\n' 'ERROR rmcp::transport::worker: MCP startup failed: handshaking with MCP server failed' >&2
  exit 19
fi

if [ "${FAKE_CODEX_MODE:-success}" = 'structured-failure' ]; then
  printf '%s\n' 'Reading additional input from stdin...' >&2
  printf '%s\n' '{"type":"item.completed","item":{"type":"error","message":"The structured failure is the real cause"}}'
  exit 20
fi

fixture_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
while IFS= read -r line || [ -n "$line" ]; do printf '%s\n' "$line"; done < "$fixture_directory/recorded-transcript.ndjson"

if [ "${FAKE_CODEX_MODE:-success}" = 'hang' ]; then
  if [ -n "${FAKE_CODEX_PID_FILE:-}" ]; then printf '%s\n' "$$" > "$FAKE_CODEX_PID_FILE"; fi
  trap 'exit 143' TERM INT
  while :; do sleep 1; done
fi

if [ "${FAKE_CODEX_MODE:-success}" = 'fail-after-thread' ]; then
  printf '%s\n' '{"type":"turn.failed","error":{"type":"invalid_request_error","code":"invalid_json_schema","message":"Invalid schema for response_format codex_output_schema: missing required property."}}'
  printf '%s\n' 'Reading additional input from stdin...' >&2
  exit 19
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
