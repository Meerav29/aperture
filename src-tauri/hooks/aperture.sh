#!/bin/sh
# Installed by aperture. Forwards Claude Code hook events to the desktop app.
# Safe to run by hand:
#   echo '{"session_id":"x","hook_event_name":"Stop"}' | ~/.claude/hooks/aperture.sh
#
# Adds two fields Claude Code doesn't send:
#   deck_pid        the claude process (this script's parent)
#   deck_host_hint  the name of claude's parent process (zsh, Code Helper, Claude, ...)
# Never fails the hook: exits 0 even if the app isn't running.

payload=$(cat)
pid=$PPID
gp=$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ')
hint=$(ps -o comm= -p "$gp" 2>/dev/null | tr -d ' "\\')

# payload is a JSON object; splice our fields in after the opening brace.
rest=${payload#*\{}
body="{\"deck_pid\":${pid:-0},\"deck_host_hint\":\"${hint}\",${rest}"

printf '%s' "$body" | curl -s -m 1 -X POST \
  -H 'Content-Type: application/json' \
  --data-binary @- "http://127.0.0.1:__PORT__/hook" >/dev/null 2>&1

exit 0
