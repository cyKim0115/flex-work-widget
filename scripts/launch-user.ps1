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
$LaunchMutexName = "Global\FlexWorkWidgetLaunch"

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
  foreach ($dir in @("src", "src-tauri\src", "src-tauri\capabilities", "scripts")) {
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
  if ($sourceAt -gt $builtAt) { return $true }
  if (Test-Path $InstallExe) {
    $installAt = (Get-Item $InstallExe).LastWriteTimeUtc
    if ($sourceAt -gt $installAt) { return $true }
  }
  return $false
}

function Test-NeedsInstallRefresh {
  if (-not (Test-Path $ReleaseExe)) { return $false }
  if (-not (Test-Path $InstallExe)) { return $true }
  $releaseHash = (Get-FileHash $ReleaseExe -Algorithm SHA256).Hash
  $installHash = (Get-FileHash $InstallExe -Algorithm SHA256).Hash
  return $releaseHash -ne $installHash
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

function Get-RunningWidgets {
  $byName = @(Get-Process -Name $ProcessName -ErrorAction SilentlyContinue)
  if ($byName.Count -gt 0) { return $byName }
  return @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
    $_.Path -and ($_.Path -ieq $InstallExe -or $_.Path -like "*\FlexWorkWidget\flex-work-widget.exe")
  })
}

function Stop-RunningWidget {
  $maxAttempts = 12
  for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
    $running = Get-RunningWidgets
    if ($running.Count -eq 0) { return }

    foreach ($proc in $running) {
      Write-Host "실행 중인 위젯 종료 (PID $($proc.Id))..."
      Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Milliseconds 500
  }

  $remaining = Get-RunningWidgets
  if ($remaining.Count -gt 0) {
    Show-Error "위젯 프로세스를 종료하지 못했습니다.`n`n작업 관리자에서 flex-work-widget을 모두 종료한 뒤 다시 시도하세요."
    exit 1
  }
}

function Install-ReleaseCopy {
  New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
  $tmp = Join-Path $InstallDir "flex-work-widget.new.exe"
  Copy-Item -Force $ReleaseExe $tmp
  if (Test-Path $InstallExe) {
    Remove-Item -Force $InstallExe -ErrorAction Stop
  }
  Rename-Item -Force $tmp (Split-Path $InstallExe -Leaf)

  $releaseHash = (Get-FileHash $ReleaseExe -Algorithm SHA256).Hash
  $installHash = (Get-FileHash $InstallExe -Algorithm SHA256).Hash
  if ($releaseHash -ne $installHash) {
    Show-Error "설치본 복사 검증에 실패했습니다. 실행 중인 위젯을 모두 종료한 뒤 다시 시도하세요."
    exit 1
  }
  Write-Host "설치 완료: $InstallExe"
}

function Start-InstalledWidget {
  if (-not (Test-Path $InstallExe)) {
    Show-Error "설치 exe를 찾을 수 없습니다: $InstallExe"
    exit 1
  }
  Start-Process -FilePath $InstallExe
  Start-Sleep -Seconds 1
  $count = @(Get-RunningWidgets).Count
  Write-Host ("Running widgets: {0}" -f $count)
  if ($count -eq 0) {
    Show-Error "위젯을 시작하지 못했습니다."
    exit 1
  }
}

$mutex = New-Object System.Threading.Mutex($false, $LaunchMutexName)
$mutexAcquired = $false
try {
  $mutexAcquired = $mutex.WaitOne(0)
  if (-not $mutexAcquired) {
    Show-Error "이미 설치 또는 재시작이 진행 중입니다. 잠시 후 다시 시도하세요."
    exit 1
  }

  $needsRebuild = Test-NeedsRebuild
  $needsInstall = Test-NeedsInstallRefresh
  $willRefresh = $ForceRebuild -or $needsRebuild -or $needsInstall

  if ($willRefresh) {
    Write-Host "기존 위젯 종료 중..."
    Stop-RunningWidget
  }

  if ($needsRebuild) {
    Build-Release
  }

  if (-not (Test-Path $ReleaseExe)) {
    Show-Error "Release exe를 찾을 수 없습니다. 먼저 npm run build:app 을 실행하세요."
    exit 1
  }

  if ($willRefresh) {
    Install-ReleaseCopy
    Remove-Item (Join-Path $InstallDir "icon.ico") -Force -ErrorAction SilentlyContinue
    Start-InstalledWidget
  } elseif (@(Get-RunningWidgets).Count -eq 0) {
    Start-InstalledWidget
  } else {
    Write-Host "최신 설치본이 이미 실행 중입니다."
  }

  try {
    $desktop = [Environment]::GetFolderPath("Desktop")
    $lnkPath = Join-Path $desktop "Flex Work Widget.lnk"
    $w = New-Object -ComObject WScript.Shell
    $lnk = $w.CreateShortcut($lnkPath)
    $lnk.TargetPath = $InstallExe
    $lnk.WorkingDirectory = $InstallDir
    $lnk.Description = "Flex 오늘 근무시간 위젯"
    $lnk.IconLocation = '{0},0' -f $InstallExe
    $lnk.Save()
  } catch {
    # ignore shortcut failures
  }

  exit 0
} finally {
  if ($mutexAcquired) {
    $mutex.ReleaseMutex() | Out-Null
  }
  $mutex.Dispose()
}
