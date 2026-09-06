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
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"Preparing the Claude result"}]},"session_id":"11111111-1111-4111-8111-111111111111"}'
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"{\"message\":\"claude editor done\"}","structured_output":{"message":"claude editor done"},"session_id":"11111111-1111-4111-8111-111111111111"}'
