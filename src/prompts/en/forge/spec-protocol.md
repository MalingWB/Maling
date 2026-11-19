From this round on, attach the Forge Spec delta to your reply. The delta is the only content the system parses; everything else is conversation.

Output rules:

1. Output this round's delta as a single fenced JSON code block in your reply; emit no other JSON outside it;
2. Include only fields changed this round; never output the full Spec;
3. The delta follows this structure (unknown fields are ignored; missing fields mean "unchanged this round"):

```json
{
  "removedStageIds": ["ids of stages to remove"],
  "name": "converged workflow name (only on rounds where it changes)",
  "intent": "one-line process intent (only on rounds where it changes)",
  "workflowName": "target workflow name for the delta when splitting into multiple workflows",
  "stages": [
    {
      "id": "id of an existing stage (omit for a new stage)",
      "role": "plan | dev | test | review | accept | custom",
      "goal": "stage goal covering the four elements",
      "roleDefinition": "full role charter for custom roles (only when role is custom; see the role-forging rules below)",
      "profileId": "system-assigned id of an already forged role — carry it verbatim; never invent, rewrite, or drop it",
      "artifact": "artifact name (when outputKind is not none)",
      "outputKind": "json | markdown | none",
      "schemaHint": { "field": "type description" },
      "successCondition": "$.field == value",
      "onFailure": "retry | manual-check | new-round | end",
      "manualCheck": false
    }
  ],
  "openQuestions": ["questions still unresolved and needing further interrogation"]
}
```

4. Each stage in `stages` is the stage's **complete new state** (not a field patch): stages align by `id` and replace wholesale; a stage without `id` is new and appended;
5. **Custom role forging rules** (when role is custom):
   - write `role` as `{"custom": {"profileName": "Role name"}}` — a concise noun phrase (2-8 chars in Chinese) naming the profession, not the process step, and never a duplicate of the built-in roles (dev/test/review/plan/accept);
   - `roleDefinition` is the role's complete charter independent of any single stage: responsibilities, working method, boundaries, output conventions — a stable professional persona reusable across workflows; do not copy the stage goal into it;
   - when the baseline stage already carries a `profileId`, that role is forged: when the user adjusts the role's name or charter in the conversation, change only `profileName`/`roleDefinition` while **carrying that `profileId` verbatim** — the system updates the role in place by id and never duplicates it;
6. `openQuestions` replaces the full unresolved list each round; output an empty array once everything converges;
7. Omit `artifact`/`schemaHint`/`successCondition` when `outputKind` is `none`; never carry an artifact contract when `onFailure` is `manual-check` (manual review and output validation are mutually exclusive);
8. A single workflow holds at most 10 stages; when splitting, carry the delta stages under a new `workflowName`.

Full constraints of the Spec fields (JSON Schema):

{{ spec_schema }}

Current baseline Spec (deltas merge onto it; empty for a new session):

{{ current_spec }}
