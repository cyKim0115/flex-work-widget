---
name: project-workflows
description: >-
  Index for shared project workflows: Korean git commits, +커푸 commit+push,
  and grouped/stepwise commits. Use when committing, drafting commit messages,
  pushing, 전체 커밋, or splitting mixed working-tree changes.
---

# Project Workflows

- `korean-git-commit` — 한글 커밋 메시지 · 커밋/푸시 절차 · `+커푸`
- `grouped-git-commit` — 비슷한 변경끼리 묶어 1~N커밋 · 「전체」커밋

## Routing

- 커밋 / 커밋 메시지 / `+커푸` / 푸시 → `korean-git-commit` (+ `git-commit-on-finish` 룰)
- 전체 커밋 / 전부 커밋 / 단계별 커밋 / 비슷한 것끼리 커밋 / 혼합 워킹트리 → `grouped-git-commit`
