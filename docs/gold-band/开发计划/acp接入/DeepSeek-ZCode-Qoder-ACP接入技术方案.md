# DeepSeek Harness / ZCode / Qoder ACP 接入技术方案

> 状态：技术方案，未实施。调研时间：2026-11-09。

## 1. 结论与范围

目标是把 DeepSeek Harness、智谱 ZCode、Qoder 加入 Agent 管理的精选 Catalog，并沿用现有 ACP client、Doctor、首次引导、Skill 同步和 Agent 管理页能力。

| Agent | 稳定 ID | 来源 | 启动模板 | 结论 |
| --- | --- | --- | --- | --- |
| DeepSeek Harness | `deepseek-harness` | 官方 CLI 本地命令 | `dsh --profile acp` | 可以接入；本机已实测 `initialize` / `session/new` |
| ZCode（智谱） | `zcode-acp` | 社区 ACP 桥接 `zcode-acp-server` | `zcode-acp-server` | 可以接入，但必须先安装桥接器和 ZCode，并保留社区项目风险标注 |
| Qoder | `qoder` | 官方 ACP Registry | `npx -y @qoder-ai/qodercli@<registry-version> --acp` | 可以接入；优先复用本机 `qoder --acp` |

不新增 provider-specific client、状态机或持久化字段。Gold Band 只维护启动模板、Skill 目录和候选入口；真实能力仍以 Doctor 的 ACP `initialize.agentCapabilities` 为准。

## 2. 现有接入链路调研

| 链路 | 当前实现 | 对新增 Agent 的含义 |
| --- | --- | --- |
| Catalog 生成 | `scripts/prepare-agent-catalog.mjs` 从官方 Registry 拉取快照，筛选 `BUILTIN_AGENT_IDS`，应用 override 与 version pin，下载 SVG | Qoder 已在官方 Registry；DeepSeek 与 ZCode 不在 Registry，需要受控补充源 |
| Rust 加载 | `resources/agent-catalog.json` 编译进 `src/agent_catalog.rs`，校验 schema、唯一 ID 与目录字段 | 只需扩展字段和数量断言，不需要新的运行时事实源 |
| 实例配置 | `ManagedAgentConfig::from_catalog` 深拷贝模板；内置 Agent 的 command/args 由 Catalog 归属，实例可覆盖 executable location | 三个 Agent 可复用创建、编辑、持久化和 workflow provider 引用 |
| 本机发现 | 未配置 Catalog 候选优先使用本地 ACP 命令或已准备 npx 缓存；只有 npx launcher 不判为已安装 | DeepSeek / ZCode 走 native command；Qoder 优先 `qoder`，否则要求已准备 npx 缓存 |
| Doctor | 启动 standalone ACP 进程，执行 `initialize`、`session/new`，有界等待 command update，再尝试 `session/delete` / `session/close` 并保存 capabilities | DeepSeek 实测能力可承接；无 command update 不是失败 |
| 会话能力 | 图片、音频、embedded context、MCP、resume/load 全部读取 Agent capabilities；缺失时降级或返回结构化错误 | 不需要按三个新 ID 写分支 |
| Skill 同步 | 主目录与兼容目录统一追加 `skills`；全局和项目作用域可拆分 | 三个 Agent 均可映射到官方或桥接器声明的用户/项目目录 |

当前存在一个必须同步修正的文档偏差：`agent-management.md` 仍写“已创建实例不跟随 Catalog command/args 变化”，而当前实现是内置 Agent 的 command/args 由 Catalog 归属并在设置序列化时恢复；知识库文档也描述为升级后跟随 Catalog。实施时必须把产品文档收敛到当前实现，避免新增内置 Agent 后继续出现两套口径。

## 3. 候选调研结果

### 3.1 DeepSeek Harness

官方入口是 `dsh --profile acp`。`dsh` 是唯一公开 Node 应用 launcher，ACP 是 profile，不是独立公共 bin。用户数据根目录默认 `~/.dsh`；Skill 文件系统提供方默认读取项目 `.dsh/skills`、项目 `.agents/skills`、用户 `~/.dsh/skills` 与用户 `~/.agents/skills`。

本机已安装 `@deepseek-ai/dsh@0.2.1-rc.2`，Node 为 `v22.22.3`。实测最小 ACP 握手结果：

- `initialize` 成功，`agentInfo.name = deepseek-harness-acp`。
- 声明 `promptCapabilities: image=false, audio=false, embeddedContext=false`。
- 声明 `mcpCapabilities.http=true`。
- 声明 `sessionCapabilities: close/list/resume`。
- `session/new` 成功，返回模型与 `reasoning_effort` 配置目录。
- `session/new` 携带 stdio MCP server 配置也能成功创建。

官方 `@deepseek-ai/dsh-acp` 包说明强调 automation-only：只输出 committed assistant text，不输出工具过程、reasoning、plan、title、usage，也不支持完整会话回放。本机 0.2.1-rc.2 的实际握手比该说明多了 `close/list/resume`。Gold Band 必须以 Doctor 实际返回为准：普通继续会话可用时走 `session/resume`；未声明 `loadSession` 时完整历史同步保持不可用。

主要体验限制：

- 首次回复可能长时间没有 tool/reasoning 流，只在 committed message 到达时出现正文；UI 不得把静默误判为失败。
- 图片、音频和 embedded resource 按路径文本降级。
- system prompt 不能通过 ACP metadata 传递，沿用现有首轮回合内隐藏上下文方案，恢复会话不重复注入。
- `session/delete` 未声明时 Doctor 先尝试 delete 再 close；实际 close 已声明，能走有界清理。

### 3.2 ZCode（智谱）

采用社区项目 `zcode-acp-server`，当前调研版本 `0.82.3`，Apache-2.0，Node >= 22。它启动真实 `zcode app-server --stdio`，不是重新实现 ZCode Agent。

推荐模板使用 native command：

```text
command: zcode-acp-server
args: []
```

前置条件：

- 用户全局安装 `zcode-acp-server`。
- ZCode CLI 可由 `ZCODE_BIN`、PATH 或桌面安装目录自动发现。
- ZCode 凭据位于 `~/.zcode/v2/config.json`。
- Node >= 22。

目录约定：主目录和项目主目录均为 `.zcode`；兼容目录暂不声明 `.agents`，待桥接器实测确认后再加入。

桥接器文档声明支持流式 diff、权限、plan、skills、MCP、auto-compaction、session resume/fork，并会把任务索引同步回 ZCode 应用。它是社区项目且迭代快，因此 Catalog 显示版本只作为 Gold Band 验证版本，不强制用户安装该版本；Doctor 负责实际可用性。

本机没有 `zcode` 和 `zcode-acp-server`。固定 `0.82.3` npx 握手在 120 秒内没有返回 `initialize`，被脚本终止；这不能证明协议不兼容，但说明缺少 ZCode 后端时可能出现长等待。选择 native command 可以让未安装桥接器在启动阶段快速失败，避免首次引导自动下载社区包后悬等到 Doctor 180 秒超时。

### 3.3 Qoder

Qoder 已在官方 ACP Registry 快照中，当前快照条目为 `qoder@0.2.14`，npx 包是 `@qoder-ai/qodercli@0.2.14 --acp`。npm 最新版本已到 `1.1.84`，说明 Registry 存在滞后；首期不得绕过 Registry 手动 pin 到 npm latest，应先使用官方 Registry 快照完成验证。若 0.2.14 不可用，优先推动或等待 Registry 更新，再重新生成快照。

启动与目录：

- npx 模板：`npx -y @qoder-ai/qodercli@<registry-version> --acp`。
- 本机优先：`qoder --acp`；`installedCommand` 为 `qoder`。
- 主目录：`.qoder`；兼容目录：`.agents`。

认证可由用户先执行 `qoder login`，或在 Agent 环境变量中配置 `QODER_PERSONAL_ACCESS_TOKEN`；Gold Band 不保存、生成或代填 token。

官方能力声明包括 Subagent、MCP、权限配置、上下文压缩和图片输入。平台支持 macOS/Linux/Windows 的 arm64 与 amd64，但 Windows ARM64 暂不支持。npm 安装路径要求 Node >= 20；当前引导策略的 Node 22/24 矩阵满足。

## 4. 数据结构与生成方案

### 4.1 本地精选补充源

新增 `resources/acp-registry-local.json`，只存放尚未进入官方 Registry、但 Gold Band 决定精选的 `deepseek-harness` 与 `zcode-acp`。文件使用与官方 Registry agent entry 兼容的结构，并额外允许 Gold Band 需要的本地覆盖字段。

构建规则：

1. 读取官方 Registry 快照或在线 Registry。
2. 读取 local supplement，按 ID 合并；官方与本地 ID 冲突时构建失败。
3. `BUILTIN_AGENT_IDS` 从 11 个扩展为 14 个，仍要求全部存在。
4. Qoder 使用官方 entry；DeepSeek 与 ZCode 使用 local entry。
5. 生成 catalog 的 source 同时记录官方 Registry 与 local supplement，避免来源谎称全部来自官方 CDN。
6. online 模式只更新官方 snapshot，不重写 local supplement。
7. offline 模式固定读取已提交官方 snapshot 与 local supplement，保证可重复构建。

这样避免把完整 Agent 定义硬编码在脚本里，也不需要为两个 Agent 引入远端私有 Registry。

### 4.2 Catalog 模板

| 字段 | DeepSeek Harness | ZCode | Qoder |
| --- | --- | --- | --- |
| id | `deepseek-harness` | `zcode-acp` | `qoder` |
| label | DeepSeek Harness | ZCode | Qoder |
| command | `dsh` | `zcode-acp-server` | `npx` |
| args | `--profile acp` | 无 | `-y @qoder-ai/qodercli@<version> --acp` |
| localLaunch | 无，command 本身就是本机命令 | 无 | `qoder --acp` |
| installedCommand | 无 | 无 | `qoder` |
| primaryAgentDir | `.dsh` | `.zcode` | `.qoder` |
| projectPrimaryAgentDir | `null`（回退 `.dsh`） | `null`（回退 `.zcode`） | `null`（回退 `.qoder`） |
| compatibleAgentDirs | `.agents` | 空 | `.agents` |
| supportsSystemPrompt | false | false | false |
| supportsExternalSessionSync | false | false | false |

三个 Agent 的全局与项目资源目录当前语义相同，先保持 `projectPrimaryAgentDir: null`，项目作用域回退到各自 `primaryAgentDir`；只有确认项目资源必须拆分时才启用拆分，不提前增加作用域模型。

### 4.3 平台排除

新增可选 Catalog 字段 `excludedPlatforms`，初始只配置 Qoder 的 `win32-arm64`。Agent 管理新增下拉和首次引导候选过滤该组合；已配置实例、workflow provider 引用和 Doctor 重试不因此被隐藏或删除。该字段来自 Qoder 官方限制，不引入通用安装器或平台猜测。

## 5. 运行时与交互方案

### 5.1 ACP client

不新增 provider-specific 分支。现有能力协商已满足：

- `promptCapabilities` 缺失时，图片/资源按路径文本降级。
- `mcpCapabilities` 只影响远程 MCP；stdio MCP 继续按协议默认传入。
- `sessionCapabilities.resume` 与顶层 `loadSession` 决定恢复策略。
- `configOptions` 支持模型与 `thought_level`。
- permission / elicitation 继续走通用卡片。

DeepSeek 的 `reasoning_effort` 属于 `category=thought_level`，可复用现有档位 remap 规则。它的模型 value 是 JSON 字符串时，前端继续展示 option name，提交原 value，不做字符串拆包。

### 5.2 Doctor 与首次引导

- DeepSeek：发现 PATH 中 `dsh` 后直接 Doctor；成功后保存默认模型与 thought level 目录。
- ZCode：只发现 PATH 中 `zcode-acp-server`；桥接器自己发现 ZCode CLI 或读取用户配置的 `ZCODE_BIN`。Gold Band 不扫描 ZCode 桌面安装目录，也不自动安装桥接器。
- Qoder：优先 PATH 中 `qoder`；没有本机命令且 npx 缓存未准备时不自动下载。用户手动在 Agent 管理页新增后，Doctor 可按现有 npx 路径准备。

Doctor 验收不要求 `available_commands_update`；无命令时返回空目录，再由 Skill 扫描补充。

### 5.3 Skill 同步

| Agent | 写入目录 | 读取目录 |
| --- | --- | --- |
| DeepSeek | `.dsh/skills` | `.dsh/skills`、`.agents/skills` |
| ZCode | `.zcode/skills` | `.zcode/skills` |
| Qoder | `.qoder/skills` | `.qoder/skills`、`.agents/skills` |

实施时必须分别验证官方 CLI 重启或 watcher 是否能发现 Gold Band 写入的 skill；不能只断言文件路径存在。

### 5.4 Agent 管理页

UI 不新增布局或新控件，只让现有 Catalog 下拉出现 14 项。图标沿用官方/项目 SVG 并下载到 `web/public/agent-icons`；品牌色图标保持原色，缺失官方授权图标时构建失败，不用占位图标冒充内置品牌。

帮助链接使用 Catalog 的 repository/website。ZCode 的描述必须能看出它是 community adapter，避免用户误以为是智谱官方 ACP 包。

## 6. 实施步骤

1. 扩展 Catalog 生成器：local supplement、14 个精选 ID、source provenance、平台排除、图标校验与测试。
2. 生成并提交 `resources/agent-catalog.json` 与图标；保留当前工作区已有 Registry 快照更新，不回退无关版本变化。
3. 扩展 Rust Catalog schema、数量断言、目录断言与平台过滤。
4. 扩展 onboarding 候选与测试：14 项、native command 优先、Qoder Windows ARM64 过滤、不自动下载未准备 Agent。
5. 补齐 Web 类型与既有 Agent 管理测试的 Catalog 断言；不改页面结构。
6. 用 mock ACP fixture 增加回归：baseline prompt 降级、resume without load、无 available commands、配置目录保留。
7. 在安装了前置条件的机器上执行真实 Doctor 与最小会话验收。
8. 同步产品文档、首次引导文档、知识库计数与支持说明。

## 7. 测试与验收

自动化测试：

- `node --test scripts/prepare-agent-catalog.test.mjs`
- `cargo test agent_catalog`
- `cargo test managed_agent_catalog`
- `cargo test onboarding`
- `pnpm --dir web test -- agent-management agent-icons onboarding`
- `pnpm --dir web exec tsc -p tsconfig.build.json`

关键断言：

1. Catalog 恰好包含 14 个精选 Agent，ID 唯一且顺序稳定。
2. Qoder 来自官方 Registry，DeepSeek/ZCode 来自 local supplement，来源冲突构建失败。
3. 三个 Agent 的 command/args、localLaunch、installedCommand、Skill 目录符合本方案。
4. Qoder 在 `win32-arm64` 不出现在新增下拉和首次引导候选中。
5. 已配置实例仍能编辑 executable location、环境变量和目录；Agent ID 不变。
6. 未安装 `dsh` / `zcode-acp-server` / `qoder` 且 npx 未准备时不触发自动下载。
7. DeepSeek fixture 的 baseline prompt、resume-without-load、无 command update 和 thought level 目录被现有 runtime 正确处理。
8. Catalog 生成失败时禁止输出残缺文件。

真实验收：

- DeepSeek：本机 `dsh` Doctor 通过；发起一个最小文本回合；停止/继续按 capabilities 行为验收；`.dsh/skills` 写入后被 CLI 识别。
- Qoder：登录后 Doctor 通过；验证模型目录、权限、图片输入声明与 `.qoder/skills`。
- ZCode：安装桥接器与 ZCode 后 Doctor 通过；验证权限、流式 diff、resume/fork 声明与 `.zcode/skills`。
- Windows x64 完整回归；Windows ARM64 确认 Qoder 隐藏且 DeepSeek/ZCode 不受影响。

## 8. 风险与取舍

| 风险 | 处理 |
| --- | --- |
| ZCode 是社区桥接器，缺失后端时 initialize 可能长等待 | 使用 native command 快速失败；首次引导不自动下载；描述标注 community |
| Qoder Registry 版本落后 npm latest | 首期信任官方 Registry；实测失败时更新 Registry 快照，不直接 pin latest |
| DeepSeek 输出 committed message，过程静默 | 复用 composer 处理中状态与 Doctor 超时；不为它伪造工具进度 |
| DeepSeek 不支持 loadSession | 禁用完整历史同步；普通 continue 按 resume capability |
| 本地补充源图标与版本维护成本 | 构建期校验 HTTPS SVG 和来源；禁止缺失图标时生成 Catalog |
| Catalog 平台限制可能被误用成安装器规则 | 字段只表达排除，不触发下载、安装或版本猜测 |

## 9. 方案自评审

过度设计评审：local supplement 和 `excludedPlatforms` 都是实际需求驱动的最小扩展。前者解决两个精选 Agent 不在官方 Registry 的来源问题，后者解决 Qoder 官方声明的 Windows ARM64 限制；不新增运行时状态机、安装器或 provider-specific adapter。

性能评审：精选 Catalog 从 11 项增加到 14 项，仍是构建期小常数数据；首屏只读内存 Catalog，不增加文件扫描或远程请求。首次引导仍只对已准备的本机候选启动有界 Doctor，不会并发下载三个 Agent。图标下载只发生在 Catalog 生成阶段。运行期无新增缓存、队列或锁。
