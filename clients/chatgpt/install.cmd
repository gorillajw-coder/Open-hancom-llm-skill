@echo off
setlocal DisableDelayedExpansion
set "OH_SETUP_CLIENT=chatgpt-desktop"
"%~dp0..\_shared\install.cmd" %*
