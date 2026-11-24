@echo off
rem M2X Studio: serve this folder on http://127.0.0.1:8765 (a secure context, so Web Serial works) and open it.
setlocal
cd /d "%~dp0"
set "URL=http://127.0.0.1:8765/index.html"
set "PY=python"
where python >nul 2>nul || set "PY=py"
start "M2X Studio server" /min %PY% -m http.server 8765 --bind 127.0.0.1
timeout /t 1 /nobreak >nul
start "" "%URL%"
endlocal
