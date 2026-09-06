@echo off
rem Installed by aperture. Forwards Claude Code hook events to the desktop app.
rem aperture_pid is not captured on Windows in the spike; host detection falls back to Unknown.
curl -s -m 1 -X POST -H "Content-Type: application/json" --data-binary @- http://127.0.0.1:__PORT__/hook >nul 2>&1
exit /b 0
