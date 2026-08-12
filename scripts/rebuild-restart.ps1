# Stops running widget, force-rebuilds release, reinstalls to %LOCALAPPDATA%, restarts.
# Used by .cursor/skills/rebuild-restart-app after implementation tasks.

$ErrorActionPreference = "Stop"
& (Join-Path $PSScriptRoot "launch-user.ps1") -ForceRebuild
exit $LASTEXITCODE
