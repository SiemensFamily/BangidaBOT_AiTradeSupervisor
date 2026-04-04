@echo off
title Crypto Scalper - Stop
echo.
echo  Stopping Crypto Scalper Bot...
wsl -e bash -c "pkill -f 'target/release/crypto-scalper' 2>/dev/null; echo Done"
echo  Bot stopped.
echo.
timeout /t 3 /nobreak > nul
