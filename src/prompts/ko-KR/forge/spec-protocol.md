이번 라운드부터 회신에 Forge Spec 증분을 첨부합니다. 증분만 시스템이 해석하며 나머지 본문은 대화용입니다.

출력 규칙:

1. 회신 본문에서 증분은 단 하나의 JSON 코드 펜스에만 출력하고, 펜스 밖에 JSON을 출력하지 않는다;
2. 이번 라운드에 변경된 필드만 포함하고, Spec 전체 출력은 금지한다(기선이 아직 없을 때, 즉 첫 라운드에서는 증분이 곧 완전한 초안이다);
3. 증분은 다음 구조를 따른다(알 수 없는 필드는 무시, 누락 필드는 "이번 라운드 불변"):

```json
{
  "removedStageIds": ["삭제할 stage id"],
  "name": "워크플로 이름(변경된 라운드에만)",
  "intent": "한 줄 프로세스 의도(변경된 라운드에만)",
  "workflowId": "수정할 기존 워크플로 id(초안에 여러 개 있으면 필수)",
  "workflowName": "해당 워크플로의 새 이름. workflowId가 없으면 새 워크플로로 분리",
  "parallelGroups": [{"id":"checks","branches":[{"id":"api","stages":["test-api"]},{"id":"ui","stages":["test-ui"]}],"maxParallel":3,"onFailure":"end"}],
  "stages": [
    {
      "id": "기존 stage의 id(새 stage는 생략)",
      "role": "plan | dev | test | review | accept | custom",
      "goal": {"input": "요구 사항", "output": "보고서", "boundary": "배포하지 않음", "acceptance": "검사 통과"},
      "roleDefinition": "custom 역할의 완전한 직책 본문(편집 가능한 custom 역할에만; 아래 사용자 정의 역할 규범 참조)",
      "artifact": "산출물 이름(outputKind가 none이 아닐 때)",
      "outputKind": "json | markdown | none",
      "schemaHint": { "필드": "타입 설명" },
      "successCondition": "$.field == value",
      "onFailure": "retry | manual-check | end | new-round",
      "manualCheck": false
    }
  ],
  "openQuestions": [
    { "question": "사용자가 결정해야 할 사항", "recommendation": "초안에 이미 채워 넣은 추천 답안" }
  ]
}
```

4. `stages`의 stage는 해당 stage의 **완전한 새 상태**(필드 패치가 아님): `id`로 "현재 기선 Spec"의 기존 stage에 맞춰 전체 교체한다. `id`가 없거나 기선에 없는 `id`는 항상 새 stage로 추가된다 — 기존 stage를 수정하려면 반드시 그 `id`를 가져가야 한다. 위치로는 정렬하지 않는다; 편집기의 목표를 그대로 둘 때는 `goal.original`을 그대로 유지합니다. 목표를 만들거나 변경할 때는 위 예시의 고정 키 네 개를 사용하고 모든 값을 사용자의 언어로 작성합니다. 편집기에 역할이 없는 단계를 유지할 때는 `role: "unassigned"`를 유지합니다. 새 단계에는 사용하지 않습니다.
5. **`onFailure`**(단계가 실패했을 때의 행선지)는 다음 중 하나다:
   - `"retry"`: 같은 단계를 다시 시도한다(다시 실행한다);
   - `"manual-check"`: 멈추고 사용자의 확인을 기다린다;
   - `"end"`: 워크플로를 종료한다;
   - `"new-round"`: 첫 단계부터 새 라운드를 시작한다;
   - `{"new-round": "stage id"}`: 지정한 단계부터 새 라운드를 시작한다;
   - `{"goto": "stage id", "continueSession": true}`: 지정한 단계로 돌아간다. `continueSession`이 true이면 그 단계는 이전 세션을 이어받아 자기 작업을 고칠 수 있다.

   점프와 재시작의 대상은 같은 워크플로에 이미 있는 stage id여야 한다;
6. **사용자 정의 역할 규범**(role이 custom일 때):
   - `role`은 `{"custom": {"profileName": "역할 이름"}}`으로 쓴다. 역할 이름은 간결한 명사구로 직업을 나타내고 프로세스 단계가 아니며, 내장 역할(개발/테스트/리뷰/기획/인수)과 이름이 겹치지 않게 한다;
   - `roleDefinition`은 이 역할이 특정 단계와 독립적인 완전한 직책 본문: 직책·작업 방법·경계·산출 습관——안정적인 전문 페르소나로 여러 워크플로에서 재사용 가능하다; stage의 goal을 roleDefinition에 복제하지 않는다;
   - 이 워크플로의 기존 역할로 변경하려면 해당 `profileName`을 사용합니다. 역할 이름을 바꿀 때는 이 역할을 쓰는 모든 단계를 함께 변경합니다. `profileId`는 출력하지 않습니다. 시스템이 연결합니다.
   - 편집할 수 있는 직무 정의가 없는 라이브러리 참조는 읽기 전용입니다. 이름을 유지하고 `roleDefinition`은 생략합니다.
7. `openQuestions`는 매 라운드 현재 미확인 항목 전체로 교체한다. 흐름의 진행을 실제로 바꾸고 사용자가 결정해야 하는 사항만 넣고, 각 항목에 추천 답안을 붙인다. 모두 수렴하면 빈 배열을 출력한다;
8. `outputKind`가 `none`이면 `artifact`/`schemaHint`/`successCondition`을 생략한다; `onFailure`가 `manual-check`이면 산출물 계약을 포함하지 않는다(수동 검토와 출력 검증은 상호 배타적);
9. 단일 워크플로의 `stages` 상한은 10이다; 분할 시 `workflowName`에 새 워크플로 이름을 지정해 증분 stages를 담는다. 한 워크플로 안의 독립 분기는 워크플로를 나누지 말고 `parallelGroups`로 표현한다. 구조나 그룹 정책이 바뀔 때만 전체 그룹 목록을 보낸다. 생략하면 유지하고 `[]`는 전부 삭제한다. 그룹의 위치는 `stages`에 처음 등장하는 구성원으로 결정된다. 기존 worker ID를 참조하며 필요한 단계는 먼저 만든다. 각 worker는 한 분기에만 속하며 분기의 `stages` 순서로 실행된다. 비어 있지 않은 분기가 두 개 이상이어야 하며 중첩은 금지한다. `maxParallel`은 1–5이며 보통 3이다. 분기 단계 실패 시 해당 분기를 끝내거나 자신 또는 같은 분기의 앞 단계를 재시도할 수 있다. 수동 검토나 새 라운드는 허용되지 않는다. 모든 분기가 끝난 후 성공이면 한 번만 다음 단계로 진행하고, 실패면 세션을 이어받지 않고 그룹 바깥의 `onFailure`를 따른다.
10. 워크플로 이름은 사용자의 모든 워크플로에서 유일해야 한다. 겹치면 저장이 거부되므로, 구별되는 이름으로 정하고 사용자에게 확인을 요청한다;
11. "현재 기선 Spec"은 모든 메시지의 실행 컨텍스트에 첨부되며 Spec의 유일한 사실 원천이다. 증분을 맞출 때 이를 읽는다. 새 stage의 이름은 필요 없다 — `id`는 시스템이 병합 후 할당하고 다음 라운드의 기선에 제시한다.

Spec 필드의 완전한 제약(JSON Schema):

{{ spec_schema }}
