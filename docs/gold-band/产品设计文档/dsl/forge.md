# Forge Spec（铸造草稿）

## 1. 一句话定义

Forge Spec 是工作流铸造的中间表示：LLM 对话收敛结论的结构化载体，经确定性编译为 `WorkflowDsl`。它是三件事的唯一事实源——对话的记忆（中断恢复）、流程图预览的数据源（图是 Spec 的投影）、编译的输入。

## 2. 结构

```jsonc
{
  "specVersion": "0.1",
  "name": "…",                 // 顶层名（单工作流时必收敛）
  "intent": "一句话流程意图",
  "sourceSkills": ["code-review"],   // 入口 B 来源 skill 引用
  "workflows": [
    {
      "name": "…",
      "stages": [
        {
          "id": "s1",           // 增量修订对齐用；新 stage 省略
          "role": "dev | test | review | plan | accept | {custom:{profileName}}",
          "goal": "…",          // 四要素：输入/产出/边界/验收
          "roleDefinition": "…", // custom 角色的职责正文（人格），与 goal 分离
          "profileId": "pf-…",  // 已铸造角色 id，系统回填、模型原样携带
          "artifact": "…", "outputKind": "json|markdown|none",
          "schemaHint": {…}, "successCondition": "$.passed == true",
          "onFailure": "retry|manual-check|new-round|end", "manualCheck": false
        }
      ]
    }
  ],
  "openQuestions": ["未收敛项"]
}
```

## 3. 关键语义

- **增量生长**：模型每轮只输出增量（`SpecDelta`），合并规则在 `src/forge/interview.rs`；全量状态由存档维护（`<task_dir>/forge/spec.json`，原子写）。
- **角色与任务分离**：`roleDefinition` 是可跨工作流复用的专业人格；`goal` 是本阶段任务。custom 角色按 `profileId` 原地更新（重编译不新建重复角色）。
- **编译**（`src/forge/compiler.rs`）：纯函数，stages→worker 节点（内置角色引用 `pf-builtin-*`），onFailure→边映射，产物 100% 过 `ValidatedWorkflow`；custom 角色在保存链路铸造/更新后回填节点引用，并为缺绑定执行槽补默认 agent 绑定。
- **四要素检测**：接受 `<输入>` 标记与「输入：」前缀两种等价形式。

## 4. 入口与回路

| 入口 | 触发 | 源注入 |
|---|---|---|
| A 会话 | Direct 会话头部锤子（Forge 会话内隐藏）；消息操作区"从此处铸造"截断 | 源会话材料（需求+用户消息，G5 预算） |
| B skill | 上下文管理页勾选；全项目级直接挂靠，含全局/内置弹项目选择 | skill 正文逐个注入隐藏区段 |
| 修改 | 工作流模板列表"对话式修改" | 反投影 Spec 作为会话初始存档基线 |

所有入口共用「预填 composer + 手动发送」交互；发送后角色固定 workflow-forge、`origin=forge`、隐藏材料注入，任务与 run 原子创建。轮完成后 runtime 提取围栏 JSON 增量合并落档，预览 Tab（流程图+卡片）事件驱动刷新。

## 5. 管理

- 铸造会话不进默认列表，由 `/chat/forge-sessions` 统一管理（Spec 摘要、试运行标记、确认式一键清理）。
- 试运行：`origin=forge-trial`，工作流截断为首节点+`$end`、`max_rounds=1`，经标准 `start_run` 启动。
- 铸造模板 `source=forged`（列表主题色 tag）；分享走 multica 工单通道。
