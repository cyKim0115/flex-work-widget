---
name: rebuild-restart-app
description: >-
  Stops the running Flex Work Widget process, force-rebuilds the Tauri
  release, reinstalls to LOCALAPPDATA, and restarts. Use when implementation
  work is done and the user wants the installed app refreshed, or when they
  mention rebuild-restart / 빌드 최신화 / 재시작 / 작업 완료 후 실행.
---

# Flex Work Widget — rebuild & restart

구현 작업이 끝난 뒤 **설치본을 최신 코드로 갱신**할 때 사용한다.

## 한 줄 실행

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File ./scripts/rebuild-restart.ps1
```

또는 `npm run rebuild:restart`

프로젝트 루트에서 실행한다. `launch-user.ps1 -ForceRebuild`를 호출한다.

## 절차 (스크립트가 수행)

1. **기존 위젯 전부 종료** (이름 + 설치 경로 기준, 실패 시 중단)
2. `npm run build:app` (VS Build Tools `vcvars64.bat` 경유)
3. release → `%LOCALAPPDATA%\FlexWorkWidget\flex-work-widget.exe` 복사 (**SHA256 검증**)
4. 설치 exe **1개**만 실행

> 빌드 **전에** 종료한다. 예전 스크립트는 빌드 후 종료라서 수십 초 동안 구 UI가 남아 “재시작이 안 된 것처럼” 보였다.

## 성공 확인 (에이전트 필수)

```powershell
Get-Process -Name flex-work-widget | Select-Object Id, Path
(Get-FileHash "$env:LOCALAPPDATA\FlexWorkWidget\flex-work-widget.exe").Hash
(Get-FileHash "src-tauri\target\release\flex-work-widget.exe").Hash
```

- 스크립트 exit code `0`
- 위젯 프로세스 **1개** (2개 이상이면 중복 실행 — 전부 종료 후 재실행)
- install / release 해시 **일치**
- release `LastWriteTime`이 방금 빌드 시각 근처

## 실패 시

- “이미 설치 또는 재시작이 진행 중” → 다른 `launch-user.ps1` / `시작.bat` 실행 중
- “프로세스를 종료하지 못함” → 작업 관리자에서 `flex-work-widget` 전부 종료 후 재시도
- Node/Rust/VS C++ Build Tools 메시지 → `npm run build:app` 단독 재시도 안내

## 에이전트 규칙

- 사용자가 **작업 완료 + 빌드/재시작**을 요청하면 이 스킬을 읽고 **직접 스크립트를 실행**한다 (안내만 하지 않음).
- 빌드는 수 분 걸릴 수 있음 — `block_until_ms`를 **600000**(10분) 이상으로 둔다.
- 성공 확인 명령까지 실행해 **해시·프로세스 수**를 검증한다.

## 관련 파일

- `scripts/rebuild-restart.ps1` — 강제 재빌드 래퍼
- `scripts/launch-user.ps1` — 빌드·설치·시작 본체 (`-ForceRebuild` 지원)
- `시작.bat` — 일반 사용자용 (`launch-user.ps1` 단일 경로)
