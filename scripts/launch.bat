@echo off
title Crypto Scalper Launcher
echo.
echo  ========================================
echo   Crypto Scalper - Starting Bot...
echo  ========================================
echo.

REM Start the bot in WSL2 background
echo [1/3] Starting bot in WSL2 (first build may take a few minutes)...
wsl -e bash -c "cd ~/BangidaBOT_AiTradeSupervisor && nohup cargo run --release > /tmp/scalper.log 2>&1 &"

REM Poll until the dashboard responds (up to 5 minutes)
echo [2/3] Waiting for dashboard to come online...
set /a tries=0
:waitloop
set /a tries+=1
if %tries% gtr 60 (
    echo.
    echo  ERROR: Dashboard did not start within 5 minutes.
    echo  Check WSL logs: wsl -e bash -c "cat /tmp/scalper.log"
    pause
    exit /b 1
)
wsl -e bash -c "curl -s -o /dev/null -w '%%{http_code}' http://localhost:3000 2>/dev/null" | findstr "200" > nul 2>&1
if errorlevel 1 (
    timeout /t 5 /nobreak > nul
    goto waitloop
)

REM Open dashboard in default browser
echo [3/3] Dashboard is ready!
start http://localhost:3000

echo.
echo  Dashboard opened at http://localhost:3000
echo  You can close this window.
echo  To stop the bot, run stop.bat
echo.
timeout /t 5 /nobreak > nul
