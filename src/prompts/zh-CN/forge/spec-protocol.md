本轮起你随回复携带 Forge Spec 增量。增量是唯一被系统解析的内容，其余正文仅供对话。

输出规则：

1. 在回复正文中，用单独一个 JSON 代码围栏输出本轮增量，围栏外不输出任何 JSON；
2. 只包含本轮发生变更的字段，禁止输出全量 Spec；
3. 增量按以下结构（未知字段会被忽略，缺失字段视为"本轮不变"）：

```json
{
  "removedStageIds": ["要删除的 stage id"],
  "name": "收敛后的工作流名（仅变更轮给出）",
  "intent": "一句话流程意图（仅变更轮给出）",
  "workflowName": "多工作流拆分时增量的目标工作流名",
  "stages": [
    {
      "id": "既有 stage 的 id（新 stage 省略此字段）",
      "role": "plan | dev | test | review | accept | custom",
      "goal": "覆盖四要素的阶段目标",
      "artifact": "产物名（outputKind 非 none 时）",
      "outputKind": "json | markdown | none",
      "schemaHint": { "字段": "类型说明" },
      "successCondition": "$.field == value",
      "onFailure": "retry | manual-check | new-round | end",
      "manualCheck": false
    }
  ],
  "openQuestions": ["本轮仍未收敛、需要继续拷打的问题"]
}
```

4. `stages` 中的 stage 是该 stage 的**完整新状态**（不是字段补丁）：按 `id` 对齐既有 stage 整体覆盖，无 `id` 视为新 stage 追加；
5. `openQuestions` 每轮整体替换为当前未收敛项全集，全部收敛后输出空数组；
6. `outputKind` 为 `none` 时省略 `artifact`/`schemaHint`/`successCondition`；`onFailure` 为 `manual-check` 时不得携带产物契约（人工复核与输出验证互斥）；
7. 单条工作流 `stages` 上限 10；需要拆分时通过 `workflowName` 指定新工作流名承载增量 stages。

Spec 字段的完整约束（JSON Schema）：

{{ spec_schema }}

当前基线 Spec（增量在此基础上合并；首轮会话基线为空）：

{{ current_spec }}
