# Stops running widget, force-rebuilds release, reinstalls to %LOCALAPPDATA%, restarts.
# Used by .cursor/skills/rebuild-restart-app after implementation tasks.

$ErrorActionPreference = "Stop"
$scriptDir = $PSScriptRoot
$launchScript = Join-Path $scriptDir "launch-user.ps1"

if (-not (Test-Path $launchScript)) {
  Write-Error "launch-user.ps1 not found: $launchScript"
  exit 1
}

& $launchScript -ForceRebuild
exit $LASTEXITCODE
