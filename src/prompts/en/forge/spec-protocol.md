From this round on, attach the Forge Spec delta to your reply. The delta is the only content the system parses; everything else is conversation.

Output rules:

1. Output this round's delta as a single fenced JSON code block in your reply; emit no other JSON outside it;
2. Include only fields changed this round; never output the full Spec (when there is no baseline yet, as in the first round, the delta is the complete draft);
3. The delta follows this structure (unknown fields are ignored; missing fields mean "unchanged this round"):

```json
{
  "removedStageIds": ["ids of stages to remove"],
  "name": "workflow name (only on rounds where it changes)",
  "intent": "one-line process intent (only on rounds where it changes)",
  "workflowId": "id of the existing workflow being revised (required for a multi-workflow draft)",
  "workflowName": "rename that workflow; without workflowId, create a new split workflow",
  "parallelGroups": [{"id":"checks","branches":[{"id":"api","stages":["test-api"]},{"id":"ui","stages":["test-ui"]}],"maxParallel":3,"onFailure":"end"}],
  "stages": [
    {
      "id": "id of an existing stage (omit for a new stage)",
      "role": "plan | dev | test | review | accept | custom",
      "goal": {"input": "Requirements", "output": "Report", "boundary": "Do not deploy", "acceptance": "Checks pass"},
      "roleDefinition": "full role charter for a custom role (for editable custom roles only; see the custom role rules below)",
      "artifact": "artifact name (when outputKind is not none)",
      "outputKind": "json | markdown | none",
      "schemaHint": { "field": "type description" },
      "successCondition": "$.field == value",
      "onFailure": "retry | manual-check | end | new-round",
      "manualCheck": false
    }
  ],
  "openQuestions": [
    { "question": "a decision the user has to make", "recommendation": "the answer you already put in the draft" }
  ]
}
```

4. Each stage in `stages` is the stage's **complete new state** (not a field patch): it aligns by `id` with an existing stage in the "Current baseline Spec" and replaces it wholesale; a stage with no `id`, or with an `id` absent from the baseline, is always a new stage and is appended — to modify an existing stage you must carry its `id`; stages never align by position; For an unchanged editor goal, copy `goal.original` verbatim; for a new or edited goal use the four fixed keys shown above, with nonempty values in the user’s language. Keep `role: "unassigned"` when retaining an editor stage that has no role; do not use it for new stages.
5. **`onFailure`** (where a stage goes when it fails) takes one of:
   - `"retry"`: retry the same stage (run it again);
   - `"manual-check"`: pause and wait for the user;
   - `"end"`: terminate the workflow;
   - `"new-round"`: start a new round from the first stage;
   - `{"new-round": "stage id"}`: start a new round from the given stage;
   - `{"goto": "stage id", "continueSession": true}`: jump back to the given stage; with `continueSession` true that stage keeps its previous session so it can fix its own work.

   A jump or restart target must be an existing stage id of the same workflow;
6. **Custom role rules** (when role is custom):
   - write `role` as `{"custom": {"profileName": "Role name"}}` — a concise noun phrase (2-8 chars in Chinese) naming the profession, not the process step, and never a duplicate of the built-in roles (dev/test/review/plan/accept);
   - `roleDefinition` is the role's complete charter independent of any single stage: responsibilities, working method, boundaries, output conventions — a stable professional persona reusable across workflows; do not copy the stage goal into it;
   - Within this workflow, use an existing `profileName` to switch a stage to that persona. Rename all its stages together to rename the persona. Never output `profileId`: the system resolves the binding.
   - Library references without an editable charter are read-only: keep the name and omit `roleDefinition`.
7. `openQuestions` replaces the full unresolved list each round: list only decisions that genuinely change how the flow runs and that the user must make, each with a recommendation; output an empty array once everything converges;
8. Omit `artifact`/`schemaHint`/`successCondition` when `outputKind` is `none`; never carry an artifact contract when `onFailure` is `manual-check` (manual review and output validation are mutually exclusive);
9. A single workflow holds at most 10 stages; when splitting, carry the delta stages under a new `workflowName`. Use `parallelGroups` for independent branches inside one workflow, not for a workflow split. Replace the entire group list only when its topology or policy changes; omit it to preserve all groups, or send `[]` to remove all. The group starts at the first member in `stages`. Reference existing worker IDs (create the workers first if necessary). Each worker belongs to only one branch; each branch lists its workers in execution order. Use at least two nonempty branches, no nesting, and `maxParallel` 1–5 (normally 3). A branch worker may end its branch or retry itself or an earlier worker in that branch, never request manual review or a new round. Once all branches settle, success advances once; otherwise the group follows its outer `onFailure` without reusing a session.
10. A workflow name must be unique among the user's workflows: saving is rejected on a clash, so pick a name that tells them apart and ask the user to confirm;
11. The "Current baseline Spec" is attached to the runtime context of every message and is the only source of truth for the Spec: read it when aligning a delta. New stages need no name from you — the system assigns their `id` after merging and shows it in the next baseline.

Full constraints of the Spec fields (JSON Schema):

{{ spec_schema }}
