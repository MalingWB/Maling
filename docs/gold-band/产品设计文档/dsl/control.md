# Gold Band Control DSL 规范

## 1. 一句话定义
Control DSL 定义 workflow 的控制面：节点之间如何流转、节点 outcome 如何映射到下一步、何时结束 run、何时打开新 round，以及 session 是否复用。

## 2. 控制原则
- 真实节点统一由 workflow node 定义（worker 或 ai-dynamic），控制层不根据节点 id 或历史名称赋予特殊语义。
- 所有跳转优先由显式 edge 表达。
- edge 的 `on` 只接受 `success / failure`。
- edge 的 `to` 可指向真实节点（worker 或 ai-dynamic）、`$end` 或 `$new-round`。
- edge 的 `session` 可选；省略时为 `new`，声明 `continue` 时目标 provider 必须支持 continue session。
- `$end` 与 `$new-round` 是控制目标，不是节点 id；`$entry` 是 `$new-round` 起点选择中的特殊值，表示当前 workflow 的 `entry`，也不是节点 id。

## 3. 全局控制字段

```json
{
  "control": {
    "max_attempts": 3,
    "max_rounds": 2
  }
}
```

两个字段均可省略，省略表示不限制。

- `max_attempts`：当前 round 内的修复/重试预算，只统计由 `failure` 触发并指向真实节点的修复跳转。比如值为 1 时，允许 `test failure -> dev` 修复一次；修复后的 `dev success -> test` 属于正常前进，不消耗次数。`output.schema` 不合法触发的隐藏追问不新增 attempt，也不消耗该预算。
- `max_rounds`：`$new-round` 可打开的新 round 最大次数，初始 round 不计入。

超过任一限制时，runtime 不再创建新的 attempt / round，当前 workflow 以 failure 结束。

## 4. edge 语义

```json
{
  "from": "test",
  "to": "dev",
  "on": "failure",
  "session": "continue"
}
```

- `from`：真实节点（worker 或 ai-dynamic）id。
- `to`：真实节点（worker 或 ai-dynamic）id、`$end` 或 `$new-round`。
- `on`：当前节点归纳出的 outcome。
- `session`：可选，`new` 或 `continue`。
- `new_round_entry`：仅当 `to="$new-round"` 时使用且必填。值为 `$entry` 或真实节点（worker 或 ai-dynamic）id，用于选择下一轮 round 的起点。

## 5. outcome 到控制决策

| outcome | 有匹配 edge | 无匹配 edge |
| --- | --- | --- |
| `success` | 按 edge 跳转；`$end` 完成成功；不能指向 `$new-round` | 等价于隐式 `success -> $end`，完成成功 |
| `failure` | 按 edge 跳转；`$end` 完成失败；`$new-round` 打开新 round | 等价于隐式 `failure -> $end`，完成失败 |
| `invalid` | 不匹配 edge；`output.schema` 不合法时先同 attempt 隐藏追问修复，修复耗尽后 workflow failure | workflow failure |
| `killed` | 不看 edge | run 完成 killed |
| `none` | 不看 edge | 暂停，等待外部继续或人工处理 |

## 6. 人工 check 与 AI 输出验证
- `manual_check=true`：worker 会话自然结束后暂停到 `WaitingForUserInput`，用户提交成功/失败后再按对应 edge 继续。
- `output + success_condition`：runtime 保存 AI 输出产物，并按成功条件归纳 outcome。
- 二者互斥；一个节点不能同时启用人工 check 和 AI 输出验证。

## 7. 新 round
`$new-round` 表示开启下一轮执行。指向 `$new-round` 的 edge 必须声明 `new_round_entry`：

```json
{
  "from": "accept",
  "to": "$new-round",
  "on": "failure",
  "new_round_entry": "$entry"
}
```

- `new_round_entry="$entry"`：下一轮从当前 workflow 的 `entry` 开始。
- `new_round_entry="<node-id>"`：下一轮从指定真实节点（worker 或 ai-dynamic）开始。

下一轮保留原始 requirement，并把上一轮触发节点的结构化输出反馈提供给新的入口节点调用。普通 worker 入口直接使用通用 `new_round_trigger` hidden context；AI-DYNAMIC 入口把同一 trigger 只投影给内部 bootstrap 的初始调用，由 bootstrap 据此重新规划本轮动态任务。

历史 task / run 可能创建于 `new_round_entry` 字段引入之前。运行启动或重跑在冻结本次 `workflow.snapshot.json` 前，会对读取到的历史 workflow 做 snapshot legacy 规范化：仅对 `to="$new-round"` 且缺失 `new_round_entry` 的边补为 `$entry`，再执行严格校验，并只把规范化结果写入本次 run 的 snapshot；`authoring/workflow.json` 不回写。运行态读取历史 frozen snapshot 时也使用同一规范化入口。作者态新建、保存 workflow 和模板时仍必须显式声明 `new_round_entry`，不能依赖该兼容默认值。

## 8. 校验要求
- `entry` 必须存在。
- 所有 edge source 必须是真实节点（worker 或 ai-dynamic）。
- edge target 必须是真实节点（worker 或 ai-dynamic）、`$end` 或 `$new-round`。
- `on=invalid` 非法；invalid 是 runtime 内部输出不合法状态，不是 workflow edge。
- `success -> $new-round` 非法；作者态目标下拉在 `on=success` 时不展示 `$new-round`。
- `to="$new-round"` 必须声明 `new_round_entry`，且值只能是 `$entry` 或已存在的真实节点（worker 或 ai-dynamic）id。
- `session=continue` 不能指向 `$end` / `$new-round`。
- `session=continue` 的目标 provider 必须支持 continue session。
- `control.max_attempts` 与 `control.max_rounds` 可省略；声明时必须为正整数。
- 启用 `success_condition` 时必须声明 JSON `output`。
- `output.artifact` 是当前节点 canonical artifact 的唯一逻辑名来源。

## 9. 固定并行组

作者可以把 `entry` 直接设成 `checks`。画布表现为开始、各条分支、配对的汇合及汇合后的节点；`$entry` 仍只表示从当前入口重开。

```json
{ "type": "parallel-group", "id": "checks", "maxParallel": 3,
  "branches": [ { "id": "api", "stages": ["unit", "contract"] }, { "id": "web", "stages": ["ui"] } ] }
```

这里的 `unit`、`contract`、`ui` 是有独立绑定的 worker；`contract` 等 `unit` 完成，不等 `ui`。一个 worker 只归属一条分支；普通节点仍只有一个成功出口。第一版不嵌套，不在分支内暂停等人工确认、开启整个工作流的新一轮或启动 AI-DYNAMIC。重试必须有预算并留在本分支内。全部分支成功，组的成功边才放行一次；耗尽预算的分支记录失败，不把其他已经成功的分支重跑。

固定分支从同一干净 Git 基线创建隔离工作目录；源目录有未提交变更时不会静默丢弃这些变更。全部成功后按分支声明顺序合成检查点；Git 不支持合并或目标已变化时保留分支现场并提示处理，不报告成功。目标身份包括分叉时的检出分支（或 detached HEAD 状态）及基线提交；即使提交号相同，切到另一条分支也不再属于原目标。下游通过 `parallel-group-result` 清单获取每支结果和终点产物定位，而不是假定所有文件都在同一目录。

分支数是流程定义，并发数是运行预算。默认候选为 3，第一版可在 1–5 范围内配置；6 条分支在上限为 5 时依然合法，第 6 条等到空位才启动。共享可写数据库、端口或账号的测试，必须隔离相关资源或设为串行。并发组不替代已有的 AI-DYNAMIC 调度设置。
