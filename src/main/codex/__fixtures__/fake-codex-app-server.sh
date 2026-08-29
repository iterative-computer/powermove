#!/bin/sh

status=${POWERMOVE_FAKE_CHATGPT_STATUS:-connected}

while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"id":%s,"result":{"userAgent":"fake-codex-app-server","codexHome":"/tmp/fake-codex","platformFamily":"unix","platformOs":"macos"}}\n' "$id"
      ;;
    *'"method":"account/read"'*)
      if [ "$status" = "disconnected" ]; then
        printf '{"id":%s,"result":{"account":null,"requiresOpenaiAuth":true}}\n' "$id"
      else
        printf '{"id":%s,"result":{"account":{"type":"chatgpt","email":"motion@example.com","planType":"pro"},"requiresOpenaiAuth":true}}\n' "$id"
      fi
      ;;
    *'"method":"account/login/start"'*)
      printf '{"id":%s,"result":{"type":"chatgpt","loginId":"login-e2e","authUrl":"https://chatgpt.com/auth/powermove-e2e"}}\n' "$id"
      ;;
    *'"method":"account/logout"'*)
      status=disconnected
      printf '{"id":%s,"result":{}}\n' "$id"
      ;;
  esac
done
