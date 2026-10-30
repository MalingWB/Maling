# WeAPM 内置角色

## 目标与边界

仅 `wb` 内部渠道用户可在自行编排的工作流节点上绑定内置 `pf-builtin-weapm`；WeAPM 依赖内部可观测性服务与网络环境。角色默认任务为可观测性查询与问题定位，使用 `weapm` CLI（`@webank/weapm-cli`）查询调用链 Trace 与日志 Log，定位慢调用、错误、超时与异常堆栈；具体流水号、子系统、DCN、IP 和时间范围来自人类指令或上游查询结果。角色展示名为 `WeAPM`，加入内置角色目录但不加入公共默认工作流；普通渠道不可见，不新增节点类型、执行器或专用授权状态机。内置角色渠道由 `src/channel.rs` 的编译期常量统一判定，不依赖用户设置或前端过滤。

全部 trace / log 查询命令均为只读，可直接执行无需逐次确认；`login` / `logout` / `config` 只影响本机 CLI 配置。唯一涉及敏感数据持久化的操作是把 UM 凭证写入共享记忆（`weapm.username` / `weapm.password`，workspace 作用域），必须先取得用户明确同意，密码任何展示场合一律 `****` 掩码。

复用现有 Profile 注册、按语言选择正文、稳定 ID 引用及 runtime 输出契约。角色属于 system prompt 中稳定执行方法；本次业务参数和目标属于 user prompt。正文以 `src/prompts/zh-CN/profile/weapm.md` 和 `src/prompts/en/profile/weapm.md` 为真源，作为静态角色关闭动态模板。

## 渠道可用范围

- 内置角色 seed 通过 `required_capability` 声明所需能力；WeAPM 为 `Some(ProfileChannelCapability::Weapm)`。`src/channel.rs` 的编译期能力矩阵是渠道适用范围的唯一真源，Profile 组合和 Prompt 正文不自行判断渠道名。
- `src/prompts/<language>/profile/weapm.md` 保持渠道中立，只描述 WeAPM 能力、输入和边界；WeAPM 仅对启用 `Weapm` 能力的渠道可见。
- 渠道值在 core crate 只有一个读取点：`src/channel.rs` 的 `RELEASE_CHANNEL`。角色列表、内置 ID 查找和默认 Profile ID 映射共用同一过滤后的目录。

## 能力与参数

角色正文自包含安装指引（环境检查、`wnpm` / 离线 `.tgz` 安装、服务地址配置、UM 登录）、全部命令用法（`trace biz`、`log subsystems/dcns/ips/files/query/below/above/stack`）、参数说明与数据分析方法，不依赖外部 skill 的本机路径。

凭证与参数通过共享记忆领域服务维护，格式与边界以 [工作空间与任务记忆](../runtime/workspace-task-memory.md) 为准：

| 参数 | Scope | 规则 |
| --- | --- | --- |
| `weapm.username` | workspace | UM 用户名；用户明确同意后长期保存、跨任务复用 |
| `weapm.password` | workspace | UM 密码；仅用户明确同意后保存；展示一律掩码 |
| `weapm.baseUrl` | workspace | 可选；默认不保存，仅用户明确要求记忆其他环境地址时保存 |

写入遵循逐 key revision/CAS：key 不存在用 `create`，存在用 `update`，删除用 `delete` 且 `entry` 为 null；冲突后重新读取核对，不盲目重试覆盖。凭证失效时先自动 `memory_read` + `weapm login` 重登一次，失败再向用户索取新凭证。

## 验收约束

- `wb` 渠道 `list_profiles` / `show_profile` 返回双语正文，`default` 及其他渠道不可见。
- 内置角色只读，修改与删除返回 `ReadonlyBuiltIn`。
- 默认工作流不自动引用 WeAPM 角色。
- 渠道目录测试覆盖 `wb` / `default` / `enterprise` / 空渠道。
