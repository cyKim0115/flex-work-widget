# Launches the standalone widget for everyday use.
# Rebuilds release when source is newer than the installed exe.
# Use -ForceRebuild to always rebuild before install (agent / post-task refresh).

param(
  [switch]$ForceRebuild
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
if (-not (Test-Path (Join-Path $Root "package.json"))) {
  $Root = $PSScriptRoot
}

$InstallDir = Join-Path $env:LOCALAPPDATA "FlexWorkWidget"
$InstallExe = Join-Path $InstallDir "flex-work-widget.exe"
$ReleaseExe = Join-Path $Root "src-tauri\target\release\flex-work-widget.exe"
$ProcessName = "flex-work-widget"

function Show-Error([string]$Message) {
  Add-Type -AssemblyName PresentationFramework | Out-Null
  [System.Windows.MessageBox]::Show($Message, "Flex Work Widget", "OK", "Error") | Out-Null
}

function Ensure-VcEnv {
  $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
  $vsPath = $null
  if (Test-Path $vswhere) {
    $vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
  }
  if (-not $vsPath) {
    $fallback = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools"
    if (Test-Path $fallback) { $vsPath = $fallback }
  }
  if (-not $vsPath) { return $null }
  $bat = Join-Path $vsPath "VC\Auxiliary\Build\vcvars64.bat"
  if (-not (Test-Path $bat)) { return $null }
  return $bat
}

function Get-SourceStamp {
  $paths = @(
    (Join-Path $Root "package.json"),
    (Join-Path $Root "index.html"),
    (Join-Path $Root "settings.html"),
    (Join-Path $Root "vite.config.ts"),
    (Join-Path $Root "src-tauri\Cargo.toml"),
    (Join-Path $Root "src-tauri\tauri.conf.json")
  )
  $latest = [datetime]::MinValue
  foreach ($p in $paths) {
    if (Test-Path $p) {
      $t = (Get-Item $p).LastWriteTimeUtc
      if ($t -gt $latest) { $latest = $t }
    }
  }
  foreach ($dir in @("src", "src-tauri\src", "src-tauri\capabilities")) {
    $full = Join-Path $Root $dir
    if (-not (Test-Path $full)) { continue }
    Get-ChildItem $full -Recurse -File -ErrorAction SilentlyContinue | ForEach-Object {
      if ($_.LastWriteTimeUtc -gt $latest) { $latest = $_.LastWriteTimeUtc }
    }
  }
  return $latest
}

function Test-NeedsRebuild {
  if ($ForceRebuild) { return $true }
  if (-not (Test-Path $ReleaseExe)) { return $true }
  $builtAt = (Get-Item $ReleaseExe).LastWriteTimeUtc
  $sourceAt = Get-SourceStamp
  return $sourceAt -gt $builtAt
}

function Build-Release {
  $vcvars = Ensure-VcEnv
  if (-not $vcvars) {
    Show-Error "Release 빌드에는 Visual Studio C++ Build Tools가 필요합니다.`n`n개발자 셸에서 한 번 실행하세요:`n  npm run build:app`n`n이후 시작.bat을 다시 실행하세요."
    exit 1
  }

  Write-Host "Release 빌드 중 (처음에는 몇 분 걸릴 수 있습니다)..."
  if (-not (Test-Path (Join-Path $Root "node_modules"))) {
    Push-Location $Root
    npm install
    Pop-Location
    if ($LASTEXITCODE -ne 0) {
      Show-Error "npm install 실패."
      exit 1
    }
  }
  $cmd = "`"$vcvars`" && cd /d `"$Root`" && npm run build:app"
  cmd /c $cmd
  if ($LASTEXITCODE -ne 0 -or -not (Test-Path $ReleaseExe)) {
    Show-Error "빌드 실패. Node.js / Rust / VS Build Tools를 확인하세요."
    exit 1
  }
}

function Stop-RunningWidget {
  Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "실행 중인 위젯 종료 (PID $($_.Id))..."
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400
  }
}

if (Test-NeedsRebuild) {
  Build-Release
}

if (-not (Test-Path $ReleaseExe)) {
  Show-Error "Release exe를 찾을 수 없습니다. 먼저 npm run build:app 을 실행하세요."
  exit 1
}

Stop-RunningWidget

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$tmp = Join-Path $InstallDir "flex-work-widget.new.exe"
Copy-Item -Force $ReleaseExe $tmp
if (Test-Path $InstallExe) { Remove-Item -Force $InstallExe }
Rename-Item -Force $tmp (Split-Path $InstallExe -Leaf)
Write-Host "설치 완료: $InstallExe"

Remove-Item (Join-Path $InstallDir "icon.ico") -Force -ErrorAction SilentlyContinue

try {
  $desktop = [Environment]::GetFolderPath("Desktop")
  $lnkPath = Join-Path $desktop "Flex Work Widget.lnk"
  $w = New-Object -ComObject WScript.Shell
  $lnk = $w.CreateShortcut($lnkPath)
  $lnk.TargetPath = $InstallExe
  $lnk.WorkingDirectory = $InstallDir
  $lnk.Description = "Flex 오늘 근무시간 위젯"
  $lnk.IconLocation = "$InstallExe,0"
  $lnk.Save()
} catch {
  # ignore shortcut failures
}

Start-Process -FilePath $InstallExe
exit 0
