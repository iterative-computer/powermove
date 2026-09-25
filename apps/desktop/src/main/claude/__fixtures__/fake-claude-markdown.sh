#!/bin/sh

if [ "$1" = '--version' ] || [ "$1" = 'auth' ]; then
  exec "$(dirname "$0")/fake-claude.sh" "$@"
fi

if [ "${FAKE_CLAUDE_MODE:-}" = 'markdown-stream' ]; then
  cat <<'JSON'
{"type":"stream_event","event":{"type":"message_start","message":{"id":"markdown"}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"## Complete\n\n**hello"}}}
{"type":"stream_event","event":{"type":"message_stop"}}
{"type":"assistant","message":{"id":"markdown","content":[{"type":"text","text":"## Complete\n\n**hello**\n\n- First\n- Second\n\nLiteral: \\*\\*example\\*\\*\n\n```js\nconst ready = true;\n```"}]}}
JSON
fi

cat <<'JSON'
{"type":"result","subtype":"success","is_error":false,"structured_output":{"summary":"## Complete\n\n**hello**\n\n- First\n- Second\n\nLiteral: \\*\\*example\\*\\*\n\n```js\nconst ready = true;\n```","commands":[],"artifacts":[],"externalActions":[],"notes":[],"extensions":[]}}
JSON
