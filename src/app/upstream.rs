//! 上游交付引用（`$upstream.completion-output`）——工作流节点 prompt 中的跨任务引用令牌。
//!
//! 令牌由外部导入的「交付拉取」角色文件携带（角色正文或 goal 中出现即视为该节点声明了拉取
//! 意图）。节点启动装配 prompt 时，[`super::node_executor`] 发现令牌 → 调用 App 上注册的
//! [`UpstreamOutputResolver`]（桌面壳注入，multica 域实现：父 issue 寻址 + 有限退避轮询）→
//! 解析结果注入首条 prompt 的隐式隐藏区段（「上游交付说明」），正文中的令牌随之移除。引擎不
//! 感知 multica：解析器缺席或解析失败 → 节点按结构化错误失败，与其他节点执行错误同路
//! （failure 边 / attempt 重试契约原样适用）。
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
/// M5-cf 语义收紧：工作流中段的 completion-output 阶段性上送只对声明了本标记的节点生效
/// （绝大多数节点无交付语义，逐节点上送会让下游在任意中间节点读到无意义的「交付」）。
/// 收尾交付不受影响——`/complete` 终态路径整体携带最后一块。桌面壳桥接在 `NodeCompleted`
/// 成功边界读 run 工作流快照 + profile 存储做门控（fail-closed：声明判定不成立即不上送）。
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
    fn resolve_upstream_output(&self, task_id: &str, node_id: &str) -> Result<String, UpstreamResolveError>;
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
    texts
        .into_iter()
        .flatten()
        .any(has_upstream_reference)
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
/// 纯谓词（数据由调用方从 run 工作流快照 + profile 存储取）；Io 侧 fail-closed：
/// 节点缺员 / 快照不可读 / profile 解析失败一律按「未声明」处理，调用方拿 `None`
/// 传入即可（误上送会让下游拿到无交付语义的中间产物，宁缺勿滥）。
pub fn declares_staged_upload(goal: Option<&str>, profile_content: Option<&str>) -> bool {
    [goal, profile_content]
        .into_iter()
        .flatten()
        .any(has_upstream_upload_declaration)
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
        assert!(!needs_upstream_resolution([Some("goal 无令牌"), Some("角色正文也没有"), None]));
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
}
