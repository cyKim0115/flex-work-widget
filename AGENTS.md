# system-crew — AGENTS

이 프로젝트는 **system-crew** 프로토콜을 사용합니다. 목표는 사용자가 준 **설명·영상·콘텐츠 참고**를 바탕으로 비슷한 게임 시스템을 구현하는 것입니다.

## 기본 원칙

1. 추측으로 바로 코딩하지 않는다. 참고 → 분해 → 스펙 → 승인 → 구현 → 충실도 검증 순서를 따른다.
2. “비슷하게”의 기준을 먼저 고정한다: **충실 재현** vs **핵심만 영감**.
3. 참고 자료에 없는 기능은 임의 추가하지 않는다. 필요하면 옵션으로 제안하고 사용자 승인을 받는다.
4. 한 번에 전체 게임을 만들지 않는다. **한 시스템(또는 수직 슬라이스)** 단위로 진행한다.
5. 한 프롬프트에 **성격이 다른 일**(분석/평가/구현/QA, 서로 다른 슬라이스 등)이 섞이면 Producer가 단계를 나누고 **현재 Stage만** 진행한다.
6. 참고 분석은 `docs/references/`에 **자산화**한다. 사용자 콜아웃과 에이전트 관찰을 분리한다.
7. 사용자가 시스템·구현 **아이디어**를 내면 평가 없이 적용하지 않는다. 실현 가능성·방향성·효율을 판단해 수용·수정·보류·거절한다 (`workflows/idea-evaluation.md`).
8. **프로토콜 밖 요청**은 4역할 루프에 끼워 맞추지 않는다. Producer가 거절·재라우팅한다 (`roles/01-producer.md`).
9. Implementer는 컴파일만으로 끝내지 않는다. 수직 슬라이스를 런타임에서 확인한 뒤 완료를 선언한다 (`roles/03-implementer.md`).
10. 검증용 **일시** 치트·스킵·디버그는 검증 후 폐기한다. 타이틀·인트로 등 진입 장벽은 `How to playtest`에 도달 방법을 적는다. 반복용 인프라는 프로젝트 스킬/영구 치트로 둔다.
11. `REJECT`는 제품 채택 거절이다. 시간 제한 실험 재시도는 새 판정으로 허용할 수 있다. 경쟁 안을 시각 비교할 때는 `exp/*` 브랜치를 나누고, 제품 머지는 비교 후 별도 `ADOPT*`로 한다 (`workflows/idea-evaluation.md`).

## 역할 라우팅

| 상황 | 담당 |
|------|------|
| 새 요청, 범위 불명, 단계 선택 | Producer |
| 프로토콜과 무관한 요청(스킬 문서화, 일상 툴링 등) | Producer → decline / 프로젝트 스킬로 재라우팅 |
| 시스템/구현 아이디어 판단 요청 | Producer (+ Analyst/Implementer 자문) |
| 영상/게임/문서 참고 분석, 스펙·자산 작성 | Systems Analyst |
| 코드·씬·프리팹 구현 | Implementer |
| 참고 대비 검수, 누락·괴리 지적 | Fidelity QA |

상세 역할: `roles/`.  
표준 루프: `workflows/from-reference.md`.  
아이디어 평가: `workflows/idea-evaluation.md`.  
자산화: `workflows/reference-assets.md`.

## 사용자 입력 예시

- “이 영상처럼 대시 후 히트스탑이 있는 콤보를 만들어줘” + URL
- “슬레이 더 스파이어식 맵 노드 선택 루프”
- “우리 게임의 X 시스템에 Y 게임의 쿨다운 UI 감각만”
- “분석만 자산화해줘. 구현은 나중에”
- “이 영상 분석·자산화하고, 상태머신 아이디어도 판단해서 괜찮으면 구현해줘” (→ mixed, Stage 분할)
- “상태머신으로 바꾸는 게 나을 것 같은데, 판단해서 적용해줘”
- “이 채팅의 MCP 검증 습관을 스킬로 남겨줘” (→ out_of_scope, 프로젝트 스킬 경로로 재라우팅)

## 이 팩을 쓰지 않는 경우

- Cursor Agent 스킬·룰·팁만 작성 (참고 게임 시스템 재현이 아님)
- 참고·스펙 없는 소규모 UI/버그픽스만 (호출어만 붙인 경우)
- CI·패키지·에디터 툴링만

## 산출물

- `docs/references/` → 참고 분석 자산
- `docs/decisions/ideas/` → 아이디어 평가 기록
- `templates/reference-asset.md` / `reference-brief.md` / `system-spec.md` / `fidelity-report.md`
- `templates/idea-evaluation.md`

<!-- system-crew:start -->
# system-crew

Shared 4-role pack (submodule: `.cursor/system-crew`).

- Update pack: `git submodule update --remote .cursor/system-crew`
- Refresh Cursor rules: `powershell -File .cursor/system-crew/scripts/sync-to-project.ps1`
- Workflow: `.cursor/system-crew/workflows/from-reference.md`
- Idea evaluation: `.cursor/system-crew/workflows/idea-evaluation.md`
- Roles: Producer → Systems Analyst → Implementer → Fidelity QA

Project-only overrides: `.cursor/rules/local/`
<!-- system-crew:end -->
