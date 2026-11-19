이번 라운드부터 회신에 Forge Spec 증분을 첨부합니다. 증분만 시스템이 해석하며 나머지 본문은 대화용입니다.

출력 규칙:

1. 회신 본문에서 증분은 단 하나의 JSON 코드 펜스에만 출력하고, 펜스 밖에 JSON을 출력하지 않는다;
2. 이번 라운드에 변경된 필드만 포함하고, Spec 전체 출력은 금지한다;
3. 증분은 다음 구조를 따른다(알 수 없는 필드는 무시, 누락 필드는 "이번 라운드 불변"):

```json
{
  "removedStageIds": ["삭제할 stage id"],
  "name": "수렴된 워크플로 이름(변경된 라운드에만)",
  "intent": "한 줄 프로세스 의도(변경된 라운드에만)",
  "workflowName": "여러 워크플로 분할 시 증분의 대상 워크플로 이름",
  "stages": [
    {
      "id": "기존 stage의 id(새 stage는 생략)",
      "role": "plan | dev | test | review | accept | custom",
      "goal": "네 요소를 포괄하는 단계 목표",
      "roleDefinition": "custom 역할의 완전한 직책 본문(role이 custom일 때만; 아래 역할 단조 규범 참조)",
      "profileId": "시스템이 부여한 기 단조 역할 id——그대로 가지고 다니며, 새로 만들거나 고쳐 쓰거나 생략하지 않는다",
      "artifact": "산출물 이름(outputKind가 none이 아닐 때)",
      "outputKind": "json | markdown | none",
      "schemaHint": { "필드": "타입 설명" },
      "successCondition": "$.field == value",
      "onFailure": "retry | manual-check | new-round | end",
      "manualCheck": false
    }
  ],
  "openQuestions": ["이번 라운드에 여전히 미수렴, 계속 심문해야 할 질문"]
}
```

4. `stages`의 stage는 해당 stage의 **완전한 새 상태**(필드 패치가 아님): `id`로 기존 stage에 맞춰 전체 교체하며, `id` 없으면 새 stage로 추가한다;
5. **사용자 정의 역할 단조 규범**(role이 custom일 때):
   - `role`은 `{"custom": {"profileName": "역할 이름"}}`으로 쓴다. 역할 이름은 간결한 명사구로 직업을 나타내고 프로세스 단계가 아니며, 내장 역할(개발/테스트/리뷰/기획/인수)과 이름이 겹치지 않게 한다;
   - `roleDefinition`은 이 역할이 특정 단계와 독립적인 완전한 직책 본문: 직책·작업 방법·경계·산출 습관——안정적인 전문 페르소나로 여러 워크플로에서 재사용 가능하다; stage의 goal을 roleDefinition에 복제하지 않는다;
   - 기선 Spec의 stage에 이미 `profileId`가 있으면 그 역할은 단조된 것이다: 사용자가 대화에서 역할 이름이나 직책을 조정하면 `profileName`/`roleDefinition`만 바꾸고 그 `profileId`를 **그대로 가지고 다닌다**. 시스템은 id로 그 역할을 제자리에서 갱신하며 중복 역할을 만들지 않는다;
6. `openQuestions`는 매 라운드 현재 미수렴 항 전체로 교체한다; 모두 수렴하면 빈 배열을 출력한다;
7. `outputKind`가 `none`이면 `artifact`/`schemaHint`/`successCondition`을 생략한다; `onFailure`가 `manual-check`이면 산출물 계약을 포함하지 않는다(수동 검토와 출력 검증은 상호 배타적);
8. 단일 워크플로의 `stages` 상한은 10이다; 분할 시 `workflowName`에 새 워크플로 이름을 지정해 증분 stages를 담는다.

Spec 필드의 완전한 제약(JSON Schema):

{{ spec_schema }}

현재 기선 Spec(증분은 이 위에 병합된다; 새 세션은 빈 상태):

{{ current_spec }}
