@echo off
setlocal DisableDelayedExpansion
set "OH_SETUP_CLIENT=claude-desktop"
"%~dp0..\_shared\install.cmd" %*
