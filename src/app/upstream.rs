//! 上游交付引用（`$upstream.completion-output`）与完成输出生产门禁——工作流节点 prompt 中的
//! 跨任务交付机制。
//!
//! # 拉取（读侧）
//!
//! 令牌由外部导入的「交付拉取」角色文件携带（角色正文或 goal 中出现即视为该节点声明了拉取
//! 意图）。节点启动装配 prompt 时，[`super::node_executor`] 发现令牌 → 调用 App 上注册的
//! [`UpstreamOutputResolver`]（桌面壳注入，multica 域实现：父 issue 寻址 + 有限退避轮询）→
//! 解析结果注入首条 prompt 的隐式隐藏区段（「上游交付说明」），正文中的令牌随之移除。引擎不
//! 感知 multica：解析器缺席或解析失败 → 节点按结构化错误失败，与其他节点执行错误同路
//! （failure 边 / attempt 重试契约原样适用）。
//!
//! # 生产（写侧，M5-cg 产出侧门禁）
//!
//! `completion-output` 块的**产出资格**在装配时按节点身份就地判定（判定数据 = 模型所见的
//! 同一份 goal / 角色正文 + 手头的工作流结构 / 动态节点身份，零反查）：工作流 sink 节点与
//! auto 验收节点 → [`CompletionProtocolDesignation::Final`]；goal / 角色正文携带
//! [`UPSTREAM_UPLOAD_MARKER`] 的节点 → [`Staged`](CompletionProtocolDesignation::Staged)；
//! 其余节点 → [`Prohibited`](CompletionProtocolDesignation::Prohibited)。指定结果经
//! [`CompletionProtocolSectionResolver`]（桌面壳 multica 域实现，按节点渲染协议变体）注入
//! 首条 prompt 隐式隐藏区段——产出被管控后，上送侧即「产出即上报」，不再有完成边界反查门控。
//!
//! 回调在 run 专用线程（`drive_accepted_run` 的 `thread::Builder::spawn`）上同步调用，允许
//! 阻塞等待；轮询预算由实现侧约束（引擎只约定「同步返回最终结果」）。

/// 上游交付引用令牌（`$` 前缀沿用 `$end` / `$new-round` 的引用语法心智）。
///
/// 令牌只是拉取声明开关：解析结果（上游交付说明块 + 上游状态行，桌面侧渲染，快照语义 =
/// 节点启动时的值）注入首条 prompt 的隐式隐藏区段，令牌本身从正文移除（按 AGENTS.md
/// system/user prompt 划分，随执行变化的交付数据属 user prompt 区，不内联进 system prompt
/// 的角色正文——内联在会话消息流里不可见、不可审计）。
pub const UPSTREAM_COMPLETION_OUTPUT_TOKEN: &str = "$upstream.completion-output";

/// 交付上传声明标记（HTML 注释，随「交付上传」角色正文携带，goal 直写等效）。
///
/// M5-cg 语义：标记是**装配时的指定声明**——节点启动装配 prompt 时对模型所见的同一份
/// goal / 角色正文做 [`declares_staged_upload`] 判定，命中即该节点收到「阶段性上送」协议
/// 变体（产出资格由运行时指定，非模型自我认定）；上送侧随产出走（节点成功 + attempt 有块
/// 即上送）。标记在判定与模型所见间零错位：喂给模型的正文含标记 ⇔ 判定为 Staged。
pub const UPSTREAM_UPLOAD_MARKER: &str = "<!-- gold-band:upstream-upload -->";

/// 令牌解析失败（结构化错误：消费端可按变体区分「配置性缺席」与「等待性超时」）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpstreamResolveError {
    /// App 未注册解析器——本地非 multica 任务选了携带令牌的角色。
    #[error("upstream output resolver is not installed (task is not bound to a remote issue)")]
    NoResolver,
    /// 任务关联的 issue 无直接父（顶层 issue / 非 issue 来源任务）。
    #[error("task has no parent issue to pull upstream output from")]
    NoParentIssue,
    /// 有限退避轮询预算内上游始终未产出交付说明。
    #[error("upstream completion output not available within the polling budget")]
    Timeout,
    /// 读取 / 网络失败（含 multica 客户端错误描述）。
    #[error("upstream resolve network failure: {0}")]
    Network(String),
}

/// 上游交付引用解析器（桌面壳 multica 域实现，App 注册）。
///
/// `task_id` 为本地任务 display id（与 lifecycle 事件同形）；实现侧据此反查在飞 multica
/// 任务映射取 workspace / issue 关联。成功返回替换文本（已含上游状态行）。
pub trait UpstreamOutputResolver: Send + Sync {
    fn resolve_upstream_output(
        &self,
        task_id: &str,
        node_id: &str,
    ) -> Result<String, UpstreamResolveError>;
}

/// 文本是否携带上游交付引用令牌。
pub(crate) fn has_upstream_reference(text: &str) -> bool {
    text.contains(UPSTREAM_COMPLETION_OUTPUT_TOKEN)
}

/// 判定一组候选文本（goal / 角色正文）是否声明了拉取意图。
pub(crate) fn needs_upstream_resolution<'a, I>(texts: I) -> bool
where
    I: IntoIterator<Item = Option<&'a str>>,
{
    texts.into_iter().flatten().any(has_upstream_reference)
}

/// 把文本中的令牌全部移除（解析结果已注入隐式隐藏区段，正文不再内联交付内容）。
pub(crate) fn strip_upstream_references(text: &str) -> String {
    text.replace(UPSTREAM_COMPLETION_OUTPUT_TOKEN, "")
}

/// 文本是否携带交付上传声明标记。
pub fn has_upstream_upload_declaration(text: &str) -> bool {
    text.contains(UPSTREAM_UPLOAD_MARKER)
}

/// 节点是否声明了交付上传职责（goal 或 profile 正文任一携带标记）。
///
/// 纯谓词。M5-cg 起在**装配时**对模型所见的同一份正文调用（判定与模型所见同源）；调用方拿
/// `None` 传入即可（正文缺员 ⇔ 模型也看不到标记 ⇔ 判定一致为否，无错位空间）。
pub fn declares_staged_upload(goal: Option<&str>, profile_content: Option<&str>) -> bool {
    [goal, profile_content]
        .into_iter()
        .flatten()
        .any(has_upstream_upload_declaration)
}

/// `completion-output` 块的节点产出指定（M5-cg 产出侧门禁）。
///
/// 由运行时在装配时按节点身份判定，经 [`CompletionProtocolSectionResolver`] 渲染为对应
/// 协议变体注入首条 prompt——模型收到的就是它自己的身份指令，不靠自我认定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionProtocolDesignation {
    /// 收尾 / 验收节点：产出就是任务的最终交付，最终回复末尾输出块。
    Final,
    /// 被指定承担阶段性上送的节点（goal / 角色正文携带 [`UPSTREAM_UPLOAD_MARKER`]）：
    /// 产出仍会被后续节点加工，仍输出块并注明阶段性快照。
    Staged,
    /// 其余节点：不得产出块（产出会被后续节点继续加工，不是交付边界）。
    Prohibited,
}

impl CompletionProtocolDesignation {
    /// 协议模板变量值（`remote_task_node_protocol.md` 的 `designation` 三分支）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Final => "final",
            Self::Staged => "staged",
            Self::Prohibited => "prohibited",
        }
    }
}

/// 工作流节点的产出指定（纯函数，判定数据全部来自装配现场）。
///
/// 判定优先级：sink（出边全指向 `$end` 或无出边——该节点终止执行链，产出即最终交付）>
/// 标记声明（[`declares_staged_upload`]，goal / 角色正文与模型所见同源）> 禁止。sink 兼具
/// 标记时按 Final 处理（产出已是最终交付，阶段性语义无意义）。
pub(crate) fn workflow_node_designation(
    workflow: &crate::dsl::ValidatedWorkflow,
    node_id: &str,
    goal: Option<&str>,
    profile_content: Option<&str>,
) -> CompletionProtocolDesignation {
    let is_sink = workflow
        .raw
        .edges
        .iter()
        .filter(|edge| edge.from == node_id)
        .all(|edge| edge.to == crate::dsl::END_NODE);
    if is_sink {
        CompletionProtocolDesignation::Final
    } else if declares_staged_upload(goal, profile_content) {
        CompletionProtocolDesignation::Staged
    } else {
        CompletionProtocolDesignation::Prohibited
    }
}

/// auto 动态叶子的产出指定（纯函数）。
///
/// 动态图的「最后一个节点」在叶子启动时不可判定（图运行时生长）——验收节点（`Acceptance`）
/// 是装配时唯一可判定的收尾产出者身份，按 Final 指定；其余叶子（Worker / Merge /
/// WorkflowInvocation）一律 Prohibited。auto 最终交付由 `/complete` 终态路径携带。
pub(crate) fn dynamic_node_designation(
    kind: crate::dynamic::DynamicNodeKind,
) -> CompletionProtocolDesignation {
    match kind {
        crate::dynamic::DynamicNodeKind::Acceptance => CompletionProtocolDesignation::Final,
        _ => CompletionProtocolDesignation::Prohibited,
    }
}

/// 完成输出协议区段解析器（桌面壳 multica 域实现，App 注册）。
///
/// 引擎在装配时算出 [`CompletionProtocolDesignation`] 后同步调用：任务关联 multica issue
/// → 按当前界面语言渲染对应协议变体，返回首条 prompt 隐式隐藏区段；本地任务 / 非 issue
/// 关联 / 解析器缺席 → `None`（无区段，不注入——与 claim 时冻结段「非 issue 不注入」同口径）。
pub trait CompletionProtocolSectionResolver: Send + Sync {
    /// `node_id`：引擎装配中的节点 display id——桌面壳据此把「该节点获准产出」的事实
    /// 记入 run 级快照（M5-cj 上传侧门禁的事实源，与模型所见同源同刻）。
    fn completion_protocol_section(
        &self,
        task_id: &str,
        node_id: &str,
        designation: CompletionProtocolDesignation,
    ) -> Option<crate::provider::PromptHiddenSection>;
}

/// 冻结区段嗅探目标：`completion-output` 协议正文的特征串（提取围栏 info 串，见
/// multica `handoff`）。M5-cg 前的 claim 时冻结段对**所有节点**写入了通用协议正文；重放
/// 旧任务时冻结段已含协议 → 不再叠加按节点变体（新旧两套产出规则并存会互相干扰）。
const FROZEN_PROTOCOL_SNIPPET: &str = "completion-output";

/// 装配时的按节点协议区段（M5-cg 产出侧门禁的注入入口）。
///
/// 两个装配面（workflow worker / auto 动态叶子）共用：先嗅探 `frozen_sections`（**仅**任务级
/// 冻结区段——上游交付说明块含父任务任意正文，混入会误伤）已含协议正文 → 跳过（旧任务照旧
/// 重放冻结文本）；否则经 App 注册的解析器取按节点变体。`SessionMode::New` 策略由调用方
/// 把关（与首条区段加载策略一致，Continue 回合不重放）。
pub(crate) fn completion_protocol_designation_section(
    app: &super::App,
    task_id: &str,
    node_id: &str,
    designation: CompletionProtocolDesignation,
    frozen_sections: &[crate::provider::PromptHiddenSection],
) -> Option<crate::provider::PromptHiddenSection> {
    if frozen_sections
        .iter()
        .any(|section| section.content.contains(FROZEN_PROTOCOL_SNIPPET))
    {
        return None;
    }
    app.completion_protocol_section(task_id, node_id, designation)
}

/// 隐式隐藏区段标题（对齐 `Gold Band remote task context` 命名惯例；RuntimeManaged
/// 合并渲染路径按内容区隔，标题仅供 RawAgent 逐区段渲染与审计）。
pub(crate) const UPSTREAM_HIDDEN_SECTION_TITLE: &str = "Gold Band upstream deliverable";

/// 把解析结果包装为首条 prompt 的隐式隐藏区段（「上游交付说明」）。
pub(crate) fn upstream_hidden_section(content: String) -> crate::provider::PromptHiddenSection {
    crate::provider::PromptHiddenSection {
        title: UPSTREAM_HIDDEN_SECTION_TITLE.to_string(),
        content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_token_in_goal_and_profile() {
        assert!(needs_upstream_resolution([Some(
            "基于上游交付开始回归测试（$upstream.completion-output）"
        )]));
        assert!(needs_upstream_resolution([
            Some("goal 无令牌"),
            Some("角色正文引用 $upstream.completion-output 处的交付说明")
        ]));
        assert!(!needs_upstream_resolution([
            Some("goal 无令牌"),
            Some("角色正文也没有"),
            None
        ]));
        assert!(!needs_upstream_resolution([None::<&str>, None]));
    }

    #[test]
    fn strips_every_token_occurrence() {
        let out = strip_upstream_references(
            "上游 A：$upstream.completion-output，再次引用 $upstream.completion-output。",
        );
        assert_eq!(out, "上游 A：，再次引用 。");
        // 无令牌文本原样返回。
        assert_eq!(strip_upstream_references("普通正文"), "普通正文");
    }

    #[test]
    fn staged_upload_declaration_requires_marker_in_goal_or_profile() {
        let marker = UPSTREAM_UPLOAD_MARKER;
        assert!(declares_staged_upload(
            Some(&format!("goal {marker}")),
            None
        ));
        assert!(declares_staged_upload(
            None,
            Some(&format!("角色正文\n{marker}\n收尾"))
        ));
        assert!(!declares_staged_upload(
            Some("goal 无标记"),
            Some("角色正文也没有")
        ));
        assert!(!declares_staged_upload(None, None));
        // 标记文本需完整：前缀片段不算声明。
        assert!(!declares_staged_upload(
            Some("<!-- gold-band:upstream-uploa"),
            None
        ));
    }

    /// 三节点链路 a → b → $end（b 为 sink；a 为中段节点）。
    fn designation_test_workflow() -> crate::dsl::ValidatedWorkflow {
        crate::dsl::validate_workflow(crate::dsl::WorkflowDsl {
            version: crate::domain::VERSION.to_string(),
            id: "workflow-designation".to_string(),
            entry: "a".to_string(),
            control: Default::default(),
            nodes: vec![
                crate::dsl::NodeDsl::Worker(crate::dsl::WorkerNode {
                    id: "a".to_string(),
                    execution_slot_id: None,
                    provider: Some("claude-acp".to_string()),
                    model: None,
                    profile: None,
                    goal: None,
                    output: None,
                    success_condition: None,
                    permission_mode: None,
                    auto_accept: false,
                    config_options: Default::default(),
                    manual_check: None,
                    prompt_envelope: crate::dsl::PromptEnvelopeMode::RuntimeManaged,
                }),
                crate::dsl::NodeDsl::Worker(crate::dsl::WorkerNode {
                    id: "b".to_string(),
                    execution_slot_id: None,
                    provider: Some("claude-acp".to_string()),
                    model: None,
                    profile: None,
                    goal: None,
                    output: None,
                    success_condition: None,
                    permission_mode: None,
                    auto_accept: false,
                    config_options: Default::default(),
                    manual_check: None,
                    prompt_envelope: crate::dsl::PromptEnvelopeMode::RuntimeManaged,
                }),
            ],
            edges: vec![
                crate::dsl::EdgeDsl {
                    from: "a".to_string(),
                    to: "b".to_string(),
                    on: crate::dsl::EdgeOutcome::Success,
                    session: None,
                    new_round_entry: None,
                },
                crate::dsl::EdgeDsl {
                    from: "b".to_string(),
                    to: crate::dsl::END_NODE.to_string(),
                    on: crate::dsl::EdgeOutcome::Success,
                    session: None,
                    new_round_entry: None,
                },
            ],
        })
        .expect("designation test workflow should validate")
    }

    #[test]
    fn workflow_node_designation_sink_marker_prohibited() {
        let workflow = designation_test_workflow();
        let marker = UPSTREAM_UPLOAD_MARKER;

        // 中段节点：无标记 → Prohibited；goal / 角色正文带标记 → Staged。
        assert_eq!(
            workflow_node_designation(&workflow, "a", None, None),
            CompletionProtocolDesignation::Prohibited
        );
        assert_eq!(
            workflow_node_designation(&workflow, "a", Some(&format!("goal {marker}")), None),
            CompletionProtocolDesignation::Staged
        );
        assert_eq!(
            workflow_node_designation(&workflow, "a", None, Some(&format!("角色\n{marker}"))),
            CompletionProtocolDesignation::Staged
        );
        // sink 节点（出边全指向 $end）→ Final；兼具标记仍 Final（产出已是最终交付）。
        assert_eq!(
            workflow_node_designation(&workflow, "b", None, None),
            CompletionProtocolDesignation::Final
        );
        assert_eq!(
            workflow_node_designation(&workflow, "b", Some(&format!("goal {marker}")), None),
            CompletionProtocolDesignation::Final
        );
    }

    #[test]
    fn workflow_node_designation_new_round_edge_is_not_sink() {
        // 出边指向 $new-round = 还有下一轮，产出会被继续加工 → 非 sink。
        let mut workflow = designation_test_workflow();
        workflow.raw.edges.retain(|edge| edge.from != "b");
        workflow.raw.edges.push(crate::dsl::EdgeDsl {
            from: "b".to_string(),
            to: crate::dsl::NEW_ROUND_NODE.to_string(),
            on: crate::dsl::EdgeOutcome::Failure,
            session: None,
            new_round_entry: Some("a".to_string()),
        });
        assert_eq!(
            workflow_node_designation(&workflow, "b", None, None),
            CompletionProtocolDesignation::Prohibited
        );
    }

    #[test]
    fn dynamic_node_designation_acceptance_is_final_others_prohibited() {
        use crate::dynamic::DynamicNodeKind;
        assert_eq!(
            dynamic_node_designation(DynamicNodeKind::Acceptance),
            CompletionProtocolDesignation::Final
        );
        for kind in [
            DynamicNodeKind::Worker,
            DynamicNodeKind::Merge,
            DynamicNodeKind::WorkflowInvocation,
        ] {
            assert_eq!(
                dynamic_node_designation(kind),
                CompletionProtocolDesignation::Prohibited
            );
        }
    }

    /// 记录调用参数的桩解析器（指定值 → 区段；未指定任务 → None 模拟本地任务）。
    struct StubProtocolResolver {
        designation: std::sync::Mutex<Vec<&'static str>>,
    }

    impl CompletionProtocolSectionResolver for StubProtocolResolver {
        fn completion_protocol_section(
            &self,
            task_id: &str,
            _node_id: &str,
            designation: CompletionProtocolDesignation,
        ) -> Option<crate::provider::PromptHiddenSection> {
            self.designation.lock().unwrap().push(designation.as_str());
            (task_id == "task-remote").then(|| crate::provider::PromptHiddenSection {
                title: "stub protocol".to_string(),
                content: format!("stub {}", designation.as_str()),
            })
        }
    }

    #[test]
    fn completion_protocol_designation_section_gates_and_sniffs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root =
            camino::Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).expect("utf8 temp path");
        let app = super::super::App::with_config(
            repo_root.clone(),
            crate::config::RuntimeConfig::default(),
        )
        .with_completion_protocol_section_resolver(std::sync::Arc::new(
            StubProtocolResolver {
                designation: std::sync::Mutex::new(Vec::new()),
            },
        ));

        let frozen_clean = vec![crate::provider::PromptHiddenSection {
            title: "Gold Band remote task context".to_string(),
            content: "附件下载失败清单".to_string(),
        }];
        // 冻结区段无协议正文 + 任务为 multica issue 任务 → 解析器区段注入。
        let section = completion_protocol_designation_section(
            &app,
            "task-remote",
            "node-sink",
            CompletionProtocolDesignation::Final,
            &frozen_clean,
        )
        .expect("stub resolver returns section");
        assert_eq!(section.title, "stub protocol");

        // 非 multica 任务（解析器返回 None）→ 不注入。
        assert!(
            completion_protocol_designation_section(
                &app,
                "task-local",
                "node-sink",
                CompletionProtocolDesignation::Final,
                &frozen_clean,
            )
            .is_none()
        );

        // 旧任务冻结段已含协议正文（M5-cg 前 claim 时定版）→ 跳过（不调用解析器叠加变体）。
        let frozen_legacy = vec![crate::provider::PromptHiddenSection {
            title: "Gold Band remote task context".to_string(),
            content: "完成输出协议……```completion-output……".to_string(),
        }];
        let resolver = std::sync::Arc::new(StubProtocolResolver {
            designation: std::sync::Mutex::new(Vec::new()),
        });
        let app_legacy = super::super::App::with_config(
            repo_root.clone(),
            crate::config::RuntimeConfig::default(),
        )
        .with_completion_protocol_section_resolver(resolver.clone());
        assert!(
            completion_protocol_designation_section(
                &app_legacy,
                "task-remote",
                "node-sink",
                CompletionProtocolDesignation::Final,
                &frozen_legacy,
            )
            .is_none()
        );
        assert!(
            resolver.designation.lock().unwrap().is_empty(),
            "frozen legacy protocol must short-circuit before the resolver"
        );

        // 解析器缺席（本地引擎默认）→ None，不注入。
        let app_bare =
            super::super::App::with_config(repo_root, crate::config::RuntimeConfig::default());
        assert!(
            completion_protocol_designation_section(
                &app_bare,
                "task-remote",
                "node-sink",
                CompletionProtocolDesignation::Final,
                &[],
            )
            .is_none()
        );
    }
}
