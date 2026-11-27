保存未通过，正在修正 {{ attempt }}/{{ max_attempts }}

<hidden data-gold-band-hidden="true" title="Forge Spec 修正">
你上一轮输出的 Forge Spec 增量合并后没有通过保存前的校验。请修复后重新输出增量，不要改变用户已经确认的结论。

结构化错误（错误码 → 明细，已一次列全）：

{{ errors }}

修复要求：

1. 只修复错误指向的 stage 与字段，其余内容保持原样；对照「当前基线 Spec」定位出错的 stage，修改时带上它的 `id`；
2. 以与之前相同的增量 JSON 结构输出修复后的增量（单一代码围栏，仍是增量、不是全量）；能修复的问题在这一轮全部修完，不要一轮只修一处；
3. 结构类错误直接修复：
   - `forge/unknown-failure-target`：失败去向指向了不存在的阶段。改成 `"retry"`，并把“这个阶段失败后应该去哪里”连同你的推荐答案写入 `openQuestions`；
   - `forge/role-id-conflict`, `forge/role-definition-conflict`: 同名角色的所有阶段使用基线中的同一份职责正文；省略 `profileId`，由系统绑定角色。
   - `forge/readonly-role`: 没有可编辑职责的角色库引用只读：保留角色名，不输出 `roleDefinition`。
4. 需要用户决定的问题不要编造答案：把它连同你的推荐答案写入 `openQuestions`，并在正文中向用户说明。这类问题包括：
   - `forge/missing-goal-element`、`forge/unresolved-questions`：阶段目标缺要素，或还有待澄清项没有落实；
   - `forge/duplicate-workflow-name`：工作流名已被占用。推荐一个能区分的新名字请用户确认，不要自行改名。
</hidden>
