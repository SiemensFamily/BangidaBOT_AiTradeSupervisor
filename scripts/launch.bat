@echo off
title Crypto Scalper Launcher
echo.
echo  ========================================
echo   Crypto Scalper - Starting Bot...
echo  ========================================
echo.

REM Start the bot in WSL2 background
echo [1/2] Starting bot in WSL2...
wsl -e bash -c "cd ~/BangidaBOT_AiTradeSupervisor && nohup cargo run --release > /tmp/scalper.log 2>&1 &"

REM Wait for the dashboard to become available
echo [2/2] Waiting for dashboard to start...
timeout /t 5 /nobreak > nul

REM Open dashboard in default browser
start http://localhost:3000

echo.
echo  Dashboard opened at http://localhost:3000
echo  You can close this window.
echo  To stop the bot, run stop.bat
echo.
timeout /t 5 /nobreak > nul
