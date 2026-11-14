@echo off
setlocal DisableDelayedExpansion
set "OH_SETUP_CLIENT=codex"
"%~dp0..\_shared\install.cmd" %*
