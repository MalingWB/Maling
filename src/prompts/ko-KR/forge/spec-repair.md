저장 미통과, 수정 진행 중 {{ attempt }}/{{ max_attempts }}

<hidden data-gold-band-hidden="true" title="Forge Spec 수복">
최근 라운드에서 출력한 Forge Spec 증분이 병합 후 저장 전 검증을 통과하지 못했습니다. 증분을 수복해 다시 출력하세요. 사용자가 이미 확인한 결론은 바꾸지 마세요.

구조화 오류(오류 코드 → 세부 내용, 전부 일괄 게재):

{{ errors }}

수복 요건:

1. 오류가 가리키는 stage와 필드만 수복하고 나머지는 원래대로 둔다. "현재 기선 Spec"에서 해당 stage를 찾아 수정 시 그 `id`를 가져간다;
2. 이전과 같은 증분 JSON 구조로 수복된 증분을 출력한다(단일 코드 펜스, 전체가 아닌 증분 그대로). 수복 가능한 문제는 이번 라운드에 모두 고치고 라운드당 하나씩 나누지 않는다;
3. 구조 계열 오류는 바로 수복한다:
   - `forge/unknown-failure-target`: 실패 시 행선지가 존재하지 않는 단계를 가리킨다. `"retry"`로 바꾸고, "이 단계가 실패하면 어디로 가야 하는가"를 추천 답안과 함께 `openQuestions`에 쓴다;
   - `forge/role-id-conflict`, `forge/role-definition-conflict`: 같은 역할을 쓰는 모든 단계의 직무 정의를 현재 기준에 맞춰 통일합니다. `profileId`는 생략하고 연결은 시스템에 맡깁니다.
   - `forge/readonly-role`: 편집할 수 있는 직무 정의가 없는 라이브러리 참조는 읽기 전용입니다. 이름을 유지하고 `roleDefinition`은 생략합니다.
4. 사용자가 결정해야 할 문제의 답을 지어내지 마세요. 추천 답안과 함께 `openQuestions`에 쓰고 본문에서 사용자에게 알린다. 해당하는 경우:
   - `forge/missing-goal-element`, `forge/unresolved-questions`: 단계 목표에 요소가 빠졌거나, 미확인 항목이 아직 반영되지 않았다;
   - `forge/duplicate-workflow-name`: 워크플로 이름이 이미 사용 중이다. 구별되는 새 이름을 추천하고 사용자에게 확인을 요청하며, 스스로 이름을 바꾸지 않는다.
</hidden>
