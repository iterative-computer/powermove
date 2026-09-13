#!/bin/sh

if [ "$1" = "--version" ]; then
  printf '%s\n' '2.1.251 (Claude Code fake)'
  exit 0
fi

if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  printf '%s\n' '{"loggedIn":true,"email":"claude@example.com","subscriptionType":"max","authMethod":"claude.ai","apiProvider":"firstParty"}'
  exit 0
fi

if [ "$1" = "auth" ] && [ "$2" = "logout" ]; then
  printf '%s\n' '{"loggedIn":false,"authMethod":"none","apiProvider":"firstParty"}'
  exit 0
fi

if [ "$1" = "auth" ] && [ "$2" = "login" ]; then
  printf '%s\n' 'Fake Claude login is waiting' >&2
  trap 'exit 143' TERM INT
  while :; do sleep 1; done
fi

printf '%s\n' '{"type":"system","subtype":"init","session_id":"11111111-1111-4111-8111-111111111111"}'
if [ "${FAKE_CLAUDE_MODE:-}" = 'hang' ]; then
  trap 'exit 143' TERM INT
  while :; do sleep 1; done
fi
printf '%s\n' '{"type":"stream_event","event":{"type":"message_start","message":{"id":"message-tool"}},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Inspecting the project"}},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tool-read","name":"Read"}},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\":\"project.json\"}"}},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_stop","index":1},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"stream_event","event":{"type":"message_stop"},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"assistant","message":{"id":"message-tool","content":[{"type":"thinking","thinking":"Inspecting the project"},{"type":"tool_use","id":"tool-read","name":"Read","input":{"file_path":"project.json"}}]},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-read","is_error":false,"content":"project contents"}]},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"stream_event","event":{"type":"message_start","message":{"id":"message-text"}},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Preparing "}},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"the Claude result"}},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_stop","index":0},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"stream_event","event":{"type":"message_stop"},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"assistant","message":{"id":"message-text","content":[{"type":"text","text":"Preparing the Claude result"}]},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"{\"message\":\"claude editor done\"}","structured_output":{"message":"claude editor done"},"session_id":"11111111-1111-4111-8111-111111111111"}'
