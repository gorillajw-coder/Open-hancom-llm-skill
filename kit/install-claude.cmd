@echo off
rem Open-hancom: register the MCP server in Claude Desktop (and Claude Code if used).
rem Double-click it, or let Claude start it. It waits until Claude Desktop is fully closed
rem (tray included), then installs, because a running app rewrites its config on exit.
rem Extra arguments go to "oh install" (e.g. --root/--workdir/--config for a test).
rem Log: %TEMP%\open-hancom-install.log. Exit: 0 ok, 1 no oh.exe, 2 registration failed,
rem 3 Claude Desktop still running after 300 checks (about 15 minutes), 4 Desktop ok but Claude Code failed.
rem Paths are only used inside quotes and never echoed, and delayed expansion stays off,
rem so "&" or "!" in a folder name cannot run or change anything.
chcp 65001 >nul
setlocal DisableDelayedExpansion
set "OH=%~dp0oh.exe"
set "LOG=%TEMP%\open-hancom-install.log"
set "OUT=%TEMP%\open-hancom-install-step.json"
set "CODE_FAILED="
> "%LOG%" echo [%date% %time%] start
>> "%LOG%" echo("%~dp0"
if not exist "%OH%" (
  echo oh.exe가 이 파일과 같은 폴더에 없습니다. zip을 모두 푼 폴더에서 실행하세요.
  >> "%LOG%" echo [error] no oh.exe
  exit /b 1
)

rem Claude Code (CLI / Code tab) keeps its own config and does not need the app closed.
set "CC_CFG=%USERPROFILE%\.claude.json"
if defined CLAUDE_CONFIG_DIR set "CC_CFG=%CLAUDE_CONFIG_DIR%\.claude.json"
if not exist "%CC_CFG%" goto desktop
"%OH%" install --client claude-code %* > "%OUT%" 2>&1
type "%OUT%" >> "%LOG%"
findstr /c:"\"verified\":true,\"warnings\"" "%OUT%" >nul
if errorlevel 1 (
  set "CODE_FAILED=1"
  >> "%LOG%" echo [fail] claude-code
) else (
  >> "%LOG%" echo [ok] claude-code
)

:desktop
set /a N=0
:wait
"%OH%" install --client claude-desktop %* > "%OUT%" 2>&1
findstr /c:"\"error\":\"client_running\"" "%OUT%" >nul
if errorlevel 1 goto installed
if %N%==0 echo Claude 앱을 트레이 아이콘까지 완전히 종료해 주세요. 종료되면 자동으로 등록합니다. 약 15분 기다립니다.
if %N%==0 (>> "%LOG%" echo [wait] claude desktop running)
set /a N+=1
if %N% GEQ 300 goto timeout
ping -n 4 127.0.0.1 >nul
goto wait

:installed
type "%OUT%" >> "%LOG%"
rem the top-level "verified" (all configs written and read back), not one of configs[].verified
findstr /c:"\"verified\":true,\"warnings\"" "%OUT%" >nul
if errorlevel 1 goto failed
>> "%LOG%" echo [ok] claude-desktop
if defined CODE_FAILED goto codefailed
echo 등록했습니다. 이제 Claude를 다시 켜세요. 기록: %%TEMP%%\open-hancom-install.log
exit /b 0

:codefailed
echo Claude Desktop에는 등록했지만 Claude Code 등록은 실패했습니다. 기록 %%TEMP%%\open-hancom-install.log 를 확인하세요.
exit /b 4

:timeout
>> "%LOG%" echo [timeout] claude desktop still running after 300 checks
echo 약 15분 동안 Claude가 종료되지 않아 멈췄습니다. Claude를 트레이까지 종료한 뒤 이 파일을 다시 실행하세요.
exit /b 3

:failed
>> "%LOG%" echo [fail] claude-desktop
echo 등록하지 못했습니다. 기록 %%TEMP%%\open-hancom-install.log 를 확인하세요.
exit /b 2
