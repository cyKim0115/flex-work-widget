---
name: korean-git-commit
description: >-
  Draft Korean git commit messages in `{영역} - {구체적 변경 내용}` format,
  and commit/push when the user ends a request with +커푸.
  Use when committing, drafting commit messages, or seeing +커푸 / 커밋 / 푸시.
---

# Korean Git Commit

Always-applied rules:

- `.cursor/rules/korean-git-commit.mdc`
- `.cursor/rules/git-commit-on-finish.mdc` (`+커푸` 포함)

## Format

```text
{영역} - {구체적 변경 내용}
```

## Rules

- Write the subject in Korean.
- Keep the first line focused on why the change exists.
- Pick a clear area label (`시스템`, `UI`, `팝업`, `데이터`, `리소스`, `연출`, `문서`, `룰`, `스킬`, `서브모듈` 등).
- Prefer one concise subject line. Add a body only when helpful.
- Use a terse noun/verb-phrase tone ending with `구현`, `반영`, `정리`, `추가`, `수정`.
- Do **not** use sentence-style endings like `~한다`, `~합니다`, `~됩니다`.
- Do **not** copy older sentence-style commits from `git log`.

## Safety protocol

1. `git status` / `git diff` / `git log`를 **병렬**로 확인
2. 시크릿(`.env`, `credentials.json` 등)은 스테이징하지 않음
3. 이번 작업 관련 파일만 stage 후 커밋
4. 커밋 후 `git status`로 확인
5. 사용자가 요청하지 않았고 `+커푸`도 없으면 push하지 않음
6. `--force` push, `--no-verify` 금지 (사용자가 명시하지 않는 한)
7. git config 변경 금지

## `+커푸` (커밋 + 푸시)

사용자 지시 **끝**에 `+커푸`가 있으면 작업을 마친 뒤:

1. 이번 작업 변경만 확인·스테이징
2. 위 형식으로 커밋
3. `git push` (필요 시 `-u origin HEAD`). **force push 금지**
4. 커밋 해시·푸시 결과를 짧게 안내

## Windows (PowerShell) 메시지

```powershell
git commit -m @"
영역 - 구체적 변경 내용

본문이 있으면 여기에
"@
```

## Examples

Good:
- `시스템 - 쿨다운 UI 피드백 수직 슬라이스 추가`
- `룰 - +커푸 커밋·푸시 지시어 반영`
- `스킬 - 관심사별 묶음 커밋 절차 추가`

Bad:
- `쿨다운 UI를 추가하고 커밋 규칙을 정리한다.`
- `Add cooldown UI`
- `WIP`

## Mixed working tree

관심사가 섞여 있거나 「전체 커밋」「전부 커밋」「단계별/비슷한 것끼리 커밋」이면 `grouped-git-commit`을 먼저 따른다. 「전체」는 빠짐없이 올리라는 뜻이지 한 커밋으로 몰아넣으라는 뜻이 아니다.

## Drafting checklist

1. Read `.cursor/rules/korean-git-commit.mdc`
2. Review staged and unstaged changes together
3. Pick one area label for the main change
4. Describe why, not only which symbols changed
5. Reject messages ending with `한다`, `합니다`, or `됩니다`
6. If `+커푸`, push after a successful commit
