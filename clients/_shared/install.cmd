@echo off
setlocal DisableDelayedExpansion
set "OH_SETUP_SOURCE=%~dp0..\..\"
set "OH_SETUP_CONFIG=%~1"
set "OH_SETUP_ROOT=%~2"
set "OH_SETUP_WORKDIR=%~3"
echo Open-hancom local MCP setup
echo Close the target desktop app before installing.
powershell -NoProfile -Command "$ErrorActionPreference='Stop'; $client=$env:OH_SETUP_CLIENT; if($client -notin @('claude-desktop','chatgpt-desktop','codex')){throw 'Start the installer in the client folder.'}; if($client -in @('chatgpt-desktop','codex')){if(Get-Process -Name ChatGPT,Codex -ErrorAction SilentlyContinue){Write-Host 'Close ChatGPT and Codex, then retry.'; exit 2}}; $exe=Join-Path $env:OH_SETUP_SOURCE 'oh.exe'; if(!(Test-Path -LiteralPath $exe)){throw 'Extract the entire release ZIP first (oh.exe, library and clients).'}; $config=$env:OH_SETUP_CONFIG; if(!$config -and $client -eq 'chatgpt-desktop'){$homeDir=if($env:CODEX_HOME){$env:CODEX_HOME}else{Join-Path $env:USERPROFILE '.codex'}; $config=Join-Path $homeDir 'config.toml'}; $argv=@('install','--client',$client,'--library',(Join-Path $env:OH_SETUP_SOURCE 'library')); if($config){$argv+=@('--config',$config)}; if($env:OH_SETUP_ROOT){$argv+=@('--root',$env:OH_SETUP_ROOT)}; if($env:OH_SETUP_WORKDIR){$argv+=@('--workdir',$env:OH_SETUP_WORKDIR)}; $raw= & $exe @argv; if($LASTEXITCODE -ne 0){$raw; exit 1}; $result=$raw|ConvertFrom-Json; if(!$result.verified){$raw; throw 'Configuration verification failed'}; Write-Host ('Client: '+$client); Write-Host ('Config: '+$result.config); Write-Host ('Backup: '+$result.backup); Write-Host ('Binary: '+$result.binary); Write-Host ('Work folder: '+$result.workdir); Write-Host 'Configuration verified. Restart the client yourself and call oh_guide to verify the connection.'"
set "OH_SETUP_EXIT=%ERRORLEVEL%"
if not "%~4"=="--no-pause" pause
exit /b %OH_SETUP_EXIT%
