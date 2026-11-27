# Agent 插件兼容性核查

范围：`resources/agent-catalog.json` 中 14 个接入项。2026-11-25 核对；宿主版本、适配器版本不是同一个版本。**没有证据的组合标记为未验证，不当作不支持，也不授权同步。** 文档中的支持不能倒推为所有已发布版本都支持。

| Gold Band 接入 | 宿主机制与分发 | 可分享的范围与证据 |
|---|---|---|
| Claude ACP | [Claude Code Plugins](https://code.claude.com/docs/en/plugins/loading.md)，marketplace 仓库、作用域设置、版本化缓存 | Claude Code 2.1.332 验证原生包，列出 `my-plugin:hello`。同一命令验证只有标准根 `plugin.json` 的目录也退出 0，但返回 `manifest: null, provides: null`；这**不是**识别标准清单的证据。|
| CodeBuddy Code | [插件参考](https://www.codebuddy.ai/docs/cli/plugins-reference)、[市场](https://www.codebuddy.ai/docs/cli/plugin-marketplaces) | 官方明确接受 `.codebuddy-plugin`、`.workbuddy-plugin`、`.claude-plugin` 清单；也接受标准 `mcp.json` 的部分语义。不能据此推断整个标准包及全部 Claude hooks 可移植。文档证据。|
| Codex ACP | [Codex 插件文档](https://developers.openai.com/plugins/build/plugins)、[宿主源码快照](https://github.com/openai/codex/tree/3f65e9f13165d85d1b1fbe20ce06c55758f927b9/codex-rs) | 宿主源码区分带标准 `$schema` 的根清单和 `.codex-plugin`、`.claude-plugin`、`.cursor-plugin` 清单；识别清单不等于执行全部厂商扩展。|
| pi ACP | [Pi Packages](https://pi.dev/packages)，包可由本地目录、npm 或 Git 分发 | pi 1.6.0 原生离线 RPC 在 Claude 格式目录内发现 `skill:hello`，来源指向 `skills/hello/SKILL.md`。说明 **Skill 可复用**，不证明 pi 解析 Claude 清单、运行其 hooks，也不是 `pi-acp` 会话验证。|
| DeepSeek Harness | [DeepSeek 官方仓库](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/claude-code-mods.md)，原生插件由宿主注册 | 官方把兼容桥描述为对部分 Claude mods/hook 模块的适配，不读取 `.claude-plugin/plugin.json`。不把桥接能力当作原生整包互通；未实测目录锁定版本。|
| ZCode ACP | [ZCode 官方插件文档](https://zcode.z.ai/cn/docs/plugin)：市场目录及其插件，可包含 skills、commands、subagents、MCP、hooks | 文档支持捆绑 `skills/<name>/SKILL.md`；未验证 `zcode-acp-server` 会加载桌面市场安装的同一插件。|
| Qoder | [CLI 插件参考](https://docs.qoder.com/cli/plugins-reference)、[CLI 发布记录](https://changelog.qoder.com/cli) | 发布记录明确区分插件安装与读取 Claude 配置的选项；后者覆盖部分 skills、commands、subagents，不能作为同一个 Claude marketplace 包可原样加载的证据。|
| Kimi | [项目所用 Kimi CLI 仓库的插件文档](https://github.com/MoonshotAI/kimi-cli/blob/main/docs/en/customization/plugins.md) | 上游有插件机制；当前目录记录的 1.52.0 未做安装/ACP 实测。不把另一仓库或更新版本的 manifest 路径硬编码进本机扫描。|
| Cursor | [插件参考](https://cursor.com/docs/reference/plugins.md)、[CLI 记录](https://cursor.com/docs/cli/changelog.md) | 支持标准与原生 `.cursor-plugin` 格式；标准 MCP 路径变量有宿主差异。CLI 能导入部分 Claude 配置，不据此承诺整包完全兼容。|
| Gemini CLI | [Extensions 参考](https://geminicli.com/docs/extensions/reference.md)，从本地或远程源安装扩展 | 官方描述 `~/.gemini/extensions/<name>/gemini-extension.json`。可共享符合目标要求的 Skill 或 MCP 服务；未验证直接加载 Claude 或标准插件包。不要将 Google 参与规范等同于 Gemini CLI 已支持。|
| Goose | [Goose 扩展目录](https://block.github.io/goose/)、[官方 MCP 示例](https://block.github.io/mcp-jupyter/docs/quickstart/) | 有基于 MCP 的扩展：可共享的是服务协议/实现，仍按目标端配置启动、鉴权。没有证据表明 Claude 插件目录可原样使用。|
| Qwen Code | [Qwen Code](https://github.com/QwenLM/qwen-code)，实测 `qwen extensions install/list` | **0.27.4** 对本地 Claude 样例输出 `Origin: Claude`，列出命令 `/hi`、Skill `hello` 和 agent `helper`；对标准样例输出 `Origin: AgentPlugins`、Skill `hello`。`--acp --experimental-skills` 的 `initialize` 成功，`session/new` 因未认证返回 `-32000`。已证实安装和识别，未证实 ACP 调用。
| OpenCode | [Plugins](https://opencode.ai/docs/plugins/)、[插件开发](https://opencode.ai/docs/build/plugins)：本地 JS/TS 模块或 npm 包 | 原生扩展是可执行模块；不能当作静态 Skill 复制或将 `plugin.json` 同名视为同一协议。未验证跨宿主整包加载。|
| Amp ACP | [Amp Plugins](https://ampcode.com/docs/customize/plugins)，Personal/Workspace Plugins | CLI `0.0.1795579235-g370682` 的 `amp plugins import --help` 明确对象为共享的 personal plugin URL，目标为插件仓库 checkout。这是 **Amp 自身的共享**，并非跨 Agent 通用；未验证社区 ACP 适配器加载。|

## WorkBuddy 与 CodeBuddy Code 的来源关系（2026-11-26）

**同一引擎不等于同一安装。** 腾讯 [CLI 安装指南](https://www.codebuddy.cn/docs/cli/installation)说明 WorkBuddy 使用 CodeBuddy 引擎；[v2.48.0 发布说明](https://www.codebuddy.cn/docs/cli/release-notes/v2.48.0)明确桌面 `.workbuddy/` 与 CLI `.codebuddy/` 分离。[桌面设置](https://www.codebuddy.cn/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/Setting)、[项目配置](https://www.codebuddy.cn/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/Project)说明代码开发模式可读取 `.codebuddy` 用户／项目配置，这不是双向安装，亦非每个插件或 hook 的兼容保证。

| 实际取得的素材 | 能确认什么 | 不应宣称什么 |
|---|---|---|
| CodeBuddy Code 的已登记插件、`.codebuddy/skills/<name>/SKILL.md` | 来源是 CLI 的安装或配置；代码开发模式可读取对应配置 | 因 `.workbuddy-plugin` 文件名就认定是桌面安装，或将 CLI 默认显示为桌面产品 |
| 用户显式选择的 WorkBuddy Skill 或专家包目录 | 可按 [Skill 规范](https://open.workbuddy.cn/docs/skill) 和[专家包规范](https://open.workbuddy.cn/docs/expert)读取声明的本地定义；专家包也可使用 `.codebuddy-plugin/plugin.json`，`agents` 指文件，`skills` 指目录 | 已在桌面安装、已启用、已在 ACP 中可调用、由 WorkBuddy 官方发行 |
| WorkBuddy 内置、市场和自建技能 | [腾讯云技能说明](https://intl.cloud.tencent.com/zh/document/product/1316/84503)区分随客户端提供的内置技能和用户技能；[桌面插件系统](https://www.codebuddy.cn/docs/workbuddy/Plugins)说明市场与选装 | 市场中可见等于已安装；所有桌面版本共用某个私有缓存/ASAR 路径；能从桌面一键导出任何内置包 |

CLI [插件参考](https://www.codebuddy.cn/docs/cli/plugins-reference)规定自定义 `skills/agents` 路径替换默认目录，不从兼容清单推断安装归属。本轮证据是官方公开资料检索和项目源码；未登录用户账号或扫描真实桌面配置。自动清单与用户指出的本地定义须分别展示。

### Windows 桌面定向发现：候选与实际核对（2026-11-27）

- 本机已做**精确路径**与版本资源核对：`D:/WorkBuddy/WorkBuddy.exe` 的产品名是 WorkBuddy，版本 `5.11.3.0`；`resources/app.asar.unpacked/resources/plugins/workbuddy-builtin/skills/expert-manager/SKILL.md` 存在。该随附包根没有 `.codebuddy-plugin`、`.workbuddy-plugin` 或 `.claude-plugin` 的 `plugin.json`。这说明它应按安装位置内的独立 Skill 定义识别，而非当作登记了一个标准插件。没有据此声称桌面实际启用了哪些功能。
- 安装位置不写死 `D:`：本机的 `Software/Classes/workbuddy/shell/open/command` 登记实际指向该 EXE。登记仅作定位，绝不执行注册命令；精确验证宿主文件和固定的随附资源相对路径。其他已知候选包括 Windows App Paths 和[腾讯列出的常见安装位置](https://cloud.tencent.com/document/faq/1831/134405)。版本不识别时保留显式选目录，不递归扫盘或解包 ASAR。
- 另一种来源是桌面用户登记 `%USERPROFILE%/.workbuddy/plugins/installed_plugins.json`，不能与 CLI 的 `.codebuddy` 登记混为一谈。[Windows 社区实例](https://cloud.tencent.com/developer/article/2768686)给出插件缓存布局；v2 是识别的登记格式而非官方永久承诺。只接受可归属的记录与缓存内匹配的包，不能拿市场目录、无登记缓存或同名文件推断已安装。自动回归使用临时登记和合成包。本次另按用户指定的目录只读核对真实登记：v2，39 条 `user` 记录，39 个对应版本缓存目录存在。只证实有登记和缓存，未读取包内正文、未逐项验证可导入素材或运行时启用状态。用户目录注册与客户端随附包是彼此独立的两条来源。非 Windows 按 `$HOME/.workbuddy/plugins` 解析符合契约的登记，不据此宣称已验证 macOS/Linux 随附包的安装布局。

## 本轮应用接入范围

除原有 Claude Code、CodeBuddy Code、Codex CLI、Qwen Code、Gemini CLI 五类安装来源外，Windows 的 WorkBuddy 桌面用户登记与安装目录随附 Skill 也可定向发现。WorkBuddy 是独立素材来源而非新增 ACP Agent；不能由此推断全部 14 个运行宿主的插件都已接入。CodeBuddy 使用登记表指向的插件缓存，不遍历市场源。Codex 根据上游 `core-plugin-common/src/installed.rs` 的规则选择 `local` 或当前版本，不能用按目录名排序的最后一个冒充当前版本。其余宿主保留上表调查结论，不拼猜测路径。

## 验收边界

- [Agent Plugins 1.0 规范](https://github.com/agentplugins/agent-plugins-spec/blob/main/spec/1.0.0.md) 定义可移植包，其中标准组件是 Skills 和 MCP servers；不规定所有宿主共用安装目录、市场、许可或启用状态。宿主私有的 hooks、命令和权限不能通过拷贝清单来保证等价。
- **分发不等于远程编排。** 市场/Git/npm 解决“从哪取得和更新包”；ACP 管理会话交互；MCP 连接本地或远端工具。远端模型并不自动拥有本地已安装插件。完整实测应在对应版本的 ACP 会话验证该组件可发现、可调用，再标为运行可用。
- Plugin 页签按包显示卡片：实际宿主的图标与可读名称、包名、版本及全局／项目作用域，下方按功能类型列出素材。自动清单是**已发现的安装**，显式添加的 WorkBuddy 目录是**手动来源**；两者都不是 ACP 会话“已加载”的状态。上表的证据等级也不是界面徽标。卡片不提供整包同步开关。只允许用户显式选择目标后同步经检查的独立 Skill；带宿主变量、依赖插件目录或依赖私有工具的 Skill 先说明依赖。不要复制凭据、注册表、自动运行的 hooks 或后台服务，不负责安装或更新插件。
- 本机可复核日志（git 忽略）：`target/verify/plugin-research/evidence-1/claude/validate.txt`、`evidence-1/web-notes.md`、`evidence-2/qwen/{10-install-claude-direct.txt,15-install-marketplace-and-agent-plugins.txt,21-acp-skills-probe.log}`、`evidence-3/{pi-11-rpc-get-commands-claude-format.log,amp-11-help-plugins_import_--help.log}`（相对于同一证据根目录）。没有使用个人账户补齐认证，也未把未完成的实验记为通过。
