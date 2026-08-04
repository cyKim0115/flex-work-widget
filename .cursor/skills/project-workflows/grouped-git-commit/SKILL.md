---
name: grouped-git-commit
description: >-
  Groups related working-tree changes into coherent git commits (one or more).
  Use when the user asks for 전체 커밋, 전부 커밋, 단계별 커밋, 비슷한 변경끼리 커밋,
  group/split commits by concern, or to commit mixed unstaged changes without
  dumping everything into one commit.
---

# Grouped Git Commit

워킹트리에 섞인 변경을 **비슷한 변경끼리** 묶어 커밋한다.

핵심은 **관련성으로 묶는 것**이다. 커밋을 무조건 2개 이상으로 나눌 필요는 없다. 한 덩어리면 1커밋으로 끝낸다.

메시지 형식·톤·`+커푸`는 `korean-git-commit`을 따른다.

## When to use

- 「전체 커밋」「전부 커밋」— 빠짐없이 올리되 **한 덩어리로 몰아넣지 않음**
- 「단계별 커밋」「비슷한 것끼리 커밋」「나눠서 커밋」
- 워킹트리에 서로 다른 관심사가 섞여 있을 때
- 「올릴 만한 것만」처럼 무관한 로컬 변경을 걸러야 할 때

## 「전체」의미

「전체」는 **남은 관련 변경을 빠짐없이** 올리라는 뜻이지, **A·B·C 기능을 한 커밋에** 올리라는 뜻이 아니다.

A, B, C가 구현되어 있으면:

1. A에 해당하는 파일만 스테이징 → `A … 구현` 메시지로 커밋
2. B, C도 같은 방식으로 반복

「A,B,C 기능 구현」 한 방 커밋은 금지한다.

## Workflow

1. `git status` / `git diff --stat` / `git log -N --oneline`를 병렬로 확인한다.
2. 변경을 **관심사 클러스터**(구현 단위)로 나눈다.
3. 클러스터가 하나면 **단일 커밋**. 둘 이상이면 **의존 순**으로 각각 스테이징·커밋한다.
4. 각 단위는 해당 파일만 경로로 스테이징한다 (`git add path1 path2` — 인터랙티브 `add -p`/`add -i` 금지).
5. 각 커밋 메시지는 `korean-git-commit` 형식 `{영역} - {구체적 변경 내용}`.
6. 전부 끝난 뒤 `git status`로 남은 파일(의도적 제외분)을 짧게 보고한다.

이미 부분 스테이징돼 있으면, 첫 클러스터 전에 `git restore --staged`로 정리한 뒤 다시 묶는다.

한 파일이 여러 단위에 걸치면 주된 단위에 넣고 메시지에 부수 변경을 짧게 밝힌다.

## How to cluster

같은 커밋에 넣을 것:

| 함께 묶기 | 예 |
|---|---|
| 한 기능의 코드+프리팹+설정 | 새 시스템 스크립트·프리팹·관련 설정 |
| 같은 데이터/테이블 세트 | 생성 스크립트 + 데이터 에셋 + 링커 |
| 같은 리소스 계열 | 폰트/머티리얼과 그걸 쓰는 UI |
| rename·삭제와 그 대체물 | 구 구현 제거 + 신 구현 + 연결 수정 |

다른 커밋으로 쪼갤 것:

| 분리 | 예 |
|---|---|
| 데이터 동기화 vs 런타임 로직 | Generated/* vs `*System.cs` |
| 도메인 로직 vs UI | Facade/System vs 팝업 |
| 월드 배치 vs UI | 씬/섬 프리팹 vs 팝업 |
| 순수 리소스 vs 기능 코드 | `Font/*.asset` vs 시스템 코드 |

애매하면 **리뷰어가 한 문장으로 설명할 수 있는지**로 판단한다. 한 문장이면 합치고, 두 문장 이상이면 나눈다.

## Exclude by default

- 로컬 렌더/에디터 설정 잡음
- `.cursor/hooks/state/*` 등 임시 상태
- 시크릿·자격증명
- 이번 요청과 무관한 워킹트리 잔여 변경

## Commit order (when splitting)

1. 데이터 / 생성물
2. 시스템·도메인 로직
3. UI·팝업·어드레서블
4. 씬/월드 배치
5. 순수 리소스

## Anti-patterns

- 「전체」요청인데 A·B·C를 한 커밋에 몰아넣기
- 한 관심사인데 억지로 여러 커밋으로 쪼개기
- 무관한 파일을 단일 커밋에 몰아넣기
- 메시지에 여러 영역을 `및`으로 나열해 혼합 커밋 숨기기
- 문장형 종결(`~한다` 등) 사용
