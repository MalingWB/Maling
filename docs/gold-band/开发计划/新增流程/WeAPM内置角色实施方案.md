# WeAPM 内置角色实施方案

## 方案

新增仅 `wb` 内部渠道提供的静态双语 `pf-builtin-weapm`，复用现有角色机制供用户绑定自定义工作流节点。参考用户提供的 `weapm-role.md`（`@webank/weapm-cli`）整理命令，不在本次执行资料中的安装、配置或查询指令。

正文默认执行可观测性查询与问题定位：按流水号查 Trace（Span 详情、耗时、状态），按子系统 / DCN / IP 查日志，通过 Trace → Log 交叉定位慢调用、错误、超时与异常堆栈。全部查询命令为只读，`login` / `logout` / `config` 只影响本机 CLI；仅 UM 凭证写共享记忆需用户明确同意。参数（流水号、子系统、DCN、IP、时间范围）只来自用户消息或上游查询结果，禁止探索式枚举；缺少时优先用结构化提问工具只问缺失项。元数据及正文中英文同步维护；WeAPM 加入 WB 内置角色目录，普通模式不可见，不新增 runtime 状态。

## 实施项

- [x] 阅读现有内置角色机制（`DEFAULT_PROFILE_SEEDS`、`built_in_profile_content`、`ProfileChannelCapability`）与 `weapm-role.md`。
- [x] 新增 `ProfileChannelCapability::Weapm`，在能力矩阵中仅 `wb` 渠道启用。
- [x] 新增 `PROFILE_WEAPM`（`localized_prompt_zh_en!`，仅 zh-CN/en）与 `profile/weapm.md` 双语正文。
- [x] 新增 `pf-builtin-weapm` seed 与 `built_in_profile_content` 分支。
- [x] 更新渠道目录测试（wb 内置角色数 10 → 11，新增 weapm 断言）。
- [x] 同步产品设计文档与开发计划。

## 验收与性能

验收覆盖中英文 list/show 返回正确正文、稳定 ID、内置只读保护、默认工作流不自动引用 WeAPM。按接口测试结果记录完成状态；不调用真实 weapm CLI 写操作（查询为只读，登录需真实凭证），不以测试通过代表平台执行通过。

过度设计审查：只维护角色资源、seed 渠道元数据和共享记忆中的既有字符串条目，复用现有构建渠道、状态、接口和命令能力，不增 runtime 数据模型、第二套子系统身份或整份配置 JSON。性能审查：内置目录最多 11 项，正文构造前过滤；正文为固定静态文本，无额外历史扫描、数据库访问、N+1、缓存、队列或无界并发。
