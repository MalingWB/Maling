//! multica issue 完成输出（completion_output）提取（issue 完成输出传递特性，码灵侧设计方案 §5）。
//!
//! 写路径协议：agent 最终回复末尾附加 info 为 `completion-output` 的代码围栏块；run 成功收尾时
//! [`extract_completion_output`] 从投影出的最终 assistant 回复中取**最后一个**匹配块，作为
//! `completion_output` 随 `POST /complete` 同事务上送（multica C6 改造，2026-10：完成与写输出
//! 单请求、后写胜，取代此前 `update_issue_status(done)` PUT 附带通道）。direct 模式由该块**门控**
//! 完成（缺块扣留，见 bridge）；工作流模式**块优先、全文兜底**（[`workflow_completion_output`]）——
//! 提取形态滑落时块内容仍随最终回复全文送达，交付不落空。

use gold_band::acp::events::{
    AcpAttemptPaths, AcpUiEvent, is_semantically_empty_agent_content, load_timeline_items,
};

/// 服务端同口径上限（multica 按 rune 计数，超长 400 且**任务不置 completed**——multica C6 改造）。
/// 客户端按 Unicode 标量预截断（截断标记计入上限），保证完成优先于内容完整。
/// 2026-09-21 服务端从 64_000 收紧为 16_000（改动清单 §1.2）。
pub(crate) const MAX_COMPLETION_OUTPUT_CHARS: usize = 16_000;

/// 预截断时追加的尾部标记（multica 改造清单 §2.2：截断到 16000 并加截断标记，标记计入上限）。
const COMPLETION_OUTPUT_TRUNCATION_MARKER: &str = "\n[已截断：超出 16000 字符上限]";

/// 从最终 assistant 回复提取交付说明块（设计方案 §5.1 规则，纯函数）。
///
/// - 找 info 串**恰为** `completion-output`（区分大小写）的代码围栏块；围栏未闭合时按 Markdown
///   语义视作延伸到文本末尾（对 agent 漏写闭合围栏鲁棒）；
/// - 任意围栏（含其他 info 串）一旦开栏即吞掉其内容——嵌在其他围栏内的 `completion-output`
///   行是围栏内容、不当成开栏（Markdown 嵌套语义）；
/// - 多个匹配块取最后一个（协议要求放末尾，取最后与指令一致且对「复述协议再产出真块」鲁棒）；
/// - 返回块内容（未 trim）；无匹配 → None。
pub(crate) fn extract_completion_output(reply: &str) -> Option<&str> {
    let mut capture: Option<(usize, usize)> = None; // (内容起始字节, 内容结束字节)
    let mut open: Option<OpenFence> = None;
    let mut line_start = 0usize;
    for line in reply.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        match &mut open {
            None => {
                if let Some((fence_len, info)) = opening_fence(bare) {
                    open = Some(OpenFence {
                        fence_len,
                        content_start: line_start + line.len(),
                        is_target: info == "completion-output",
                    });
                }
            }
            Some(fence) => {
                if is_closing_fence(bare, fence.fence_len) {
                    if fence.is_target {
                        capture = Some((fence.content_start, line_start));
                    }
                    open = None;
                }
            }
        }
        line_start += line.len();
    }
    // 围栏未闭合 → 按 Markdown 语义延伸到文本末尾（在最后的匹配块上优先）。
    if let Some(fence) = open.filter(|fence| fence.is_target) {
        capture = Some((fence.content_start, reply.len()));
    }
    capture.map(|(start, end)| &reply[start..end])
}

/// 开栏状态（任意 info 串都跟踪，防其他围栏内容里的 `completion-output` 行误判为开栏）。
struct OpenFence {
    fence_len: usize,
    content_start: usize,
    is_target: bool,
}

/// 行首 ``` 围栏（≥3 个反引号，允许 ≤3 空格缩进）。返回 (围栏长度, info 串)。
fn opening_fence(line: &str) -> Option<(usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let fence_len = rest.chars().take_while(|&c| c == '`').count();
    if fence_len < 3 {
        return None;
    }
    Some((fence_len, rest[fence_len..].trim()))
}

/// 闭合围栏：仅反引号（可带尾随空白），长度 ≥ 开栏长度。
fn is_closing_fence(line: &str, fence_len: usize) -> bool {
    let trimmed = line.trim();
    let len = trimmed.chars().filter(|&c| c == '`').count();
    len >= fence_len && len == trimmed.chars().count()
}

/// trim + 空判 + 16k 预截断（超长时尾部追加截断标记，标记计入上限——发送值恒 ≤16_000 rune，
/// 规避 server 超长 400「任务不置 completed」的硬失败）。trim 后为空 → None（空值不发送，
/// 服务端「缺省/空串不动现值」语义决定）。
fn clamp_completion_output(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= MAX_COMPLETION_OUTPUT_CHARS {
        return Some(trimmed.to_string());
    }
    let kept = MAX_COMPLETION_OUTPUT_CHARS - COMPLETION_OUTPUT_TRUNCATION_MARKER.chars().count();
    let mut clamped: String = trimmed.chars().take(kept).collect();
    clamped.push_str(COMPLETION_OUTPUT_TRUNCATION_MARKER);
    Some(clamped)
}

/// 提取块 + trim + 16k 预截断（设计方案 §5.1 组合入口，direct 门控路径调用）。
pub(crate) fn completion_output_from_reply(reply: &str) -> Option<String> {
    extract_completion_output(reply).and_then(clamp_completion_output)
}

/// 工作流收尾采集（multica C6 改造清单 §2.1）：**块优先、最终回复全文兜底**。
///
/// 收尾节点按协议产出 `completion-output` 围栏块 → 只传块（干净、有意图的交付物，非节点名/
/// 任务摘要）；提不出块（未产出/近似形态滑落，如 §12.55 的 info 串另起一行）→ 传最终回复
/// **全文**（预截断）——块内容原样嵌在全文里送达，交付不落空。
pub(crate) fn workflow_completion_output(reply: &str) -> Option<String> {
    completion_output_from_reply(reply).or_else(|| clamp_completion_output(reply))
}

/// 近似形态 1：info 串 `completion-output` 被写在裸围栏（或其它 info 围栏）开栏后的新行——
/// 合法形态必须与 ``` 同行（GFM info-string 语义，与提取器 [`opening_fence`] 同口径）。
pub(crate) const NEAR_MISS_INFO_ON_OWN_LINE: &str = "info_on_own_line_after_fence";
/// 近似形态 2：围栏 info 串包含 `completion-output` 但不精确相等（如 `completion-output:`）。
pub(crate) const NEAR_MISS_SIMILAR_INFO_STRING: &str = "fence_info_contains_target";

/// 完成块探测结果：`block` 为提取命中（trim + 16k 截断后）的交付说明；块缺失且回复中存在
/// 「近似 completion-output 围栏」时 `near_miss_reason` 给出机器可检索的原因 slug。
/// `near_miss_reason` **仅作日志诊断、不参与门控判定**——门控仍只看 `block`（fail-closed：
/// 宁可保持进行中，不可把形态不符的块误当完成）。
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CompletionBlockProbe {
    pub(crate) block: Option<String>,
    pub(crate) near_miss_reason: Option<&'static str>,
}

/// 提取 + 近似形态探测（门控日志诊断用，纯函数；开发设计 §12.55 根因修复）。
///
/// 背景：内网实测 agent 首次真实输出块时把 info 串写到了裸围栏的下一行，提取正确拒绝
/// （GFM 语义）但与「完全未输出块」不可区分，任务静默卡在进行中、排查需导出 timeline
/// 取证。探测补上这层可见性；块命中时无需诊断。
pub(crate) fn completion_block_probe(reply: &str) -> CompletionBlockProbe {
    let block = completion_output_from_reply(reply);
    let near_miss_reason = if block.is_some() {
        None
    } else {
        near_miss_completion_fence(reply)
    };
    CompletionBlockProbe {
        block,
        near_miss_reason,
    }
}

/// 近似完成围栏检测（纯函数）：只识别**结构化**的围栏形态滑落——(1) 围栏 info 串包含
/// `completion-output` 但不精确相等；(2) 裸围栏（或其它 info 围栏）开栏后的首个非空行
/// 恰为 `completion-output`（info 串被写到了新行）。纯 prose 提及该串（如 agent 解释
/// 「未完成不得输出块」）**不**触发——按协议正常扣留的轮次不得误报成格式错误。
///
/// 其它围栏内容首行恰好是该串的极端构造可能误报——仅影响日志级别、不影响门控行为，可接受。
fn near_miss_completion_fence(reply: &str) -> Option<&'static str> {
    let lines: Vec<&str> = reply.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let Some((_, info)) = opening_fence(line) else {
            continue;
        };
        if info == "completion-output" {
            continue;
        }
        if info.contains("completion-output") {
            return Some(NEAR_MISS_SIMILAR_INFO_STRING);
        }
        let next_content_line = lines[index + 1..]
            .iter()
            .map(|line| line.trim())
            .find(|line| !line.is_empty());
        if next_content_line == Some("completion-output") {
            return Some(NEAR_MISS_INFO_ON_OWN_LINE);
        }
    }
    None
}

/// 投影 run 最终 attempt 的最终 assistant 回复文本（设计方案 §5.2 支撑改动 2）。
///
/// `acp.timeline.jsonl` 中同流 textDelta 已由 runtime 合并为单条目（`apply_streaming_delta`，
/// 内容累积），`load_timeline_items` 物化后按 seq 排序——取**最后一条**非占位 textDelta 的 content
/// 即最终 assistant 消息。读失败/无文本 → None（fail-open，调用方仍照常流转 issue）。
pub(crate) fn final_assistant_reply(attempt_dir: &str) -> Option<String> {
    let paths = AcpAttemptPaths::from_attempt_dir(attempt_dir.into());
    let items = match load_timeline_items(&paths.timeline) {
        Ok(items) => items,
        Err(error) => {
            tracing::warn!(
                timeline = %paths.timeline,
                %error,
                "multica completion-output: timeline read failed (fail-open)"
            );
            return None;
        }
    };
    items
        .iter()
        .filter(|item| is_assistant_text(item))
        .next_back()
        .and_then(|item| item.content.clone())
        .filter(|content| !content.trim().is_empty())
}

/// 最终 assistant 文本判据：textDelta 且非占位（复用 runtime 的语义空判据，保持与 UI 投影同源）。
fn is_assistant_text(item: &AcpUiEvent) -> bool {
    item.kind == "textDelta" && !is_semantically_empty_agent_content(item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

    #[test]
    fn extract_hits_completion_output_block() {
        let reply = "工作完成。\n\n```completion-output\n部署地址: http://x\n自测结论: 通过\n```\n";
        assert_eq!(
            extract_completion_output(reply),
            Some("部署地址: http://x\n自测结论: 通过\n")
        );
    }

    #[test]
    fn extract_ignores_other_info_strings_and_plain_fences() {
        let reply = "```json\n{\"a\":1}\n```\n```Completion-Output\n大小写敏感\n```\n普通说明";
        assert_eq!(extract_completion_output(reply), None);
    }

    #[test]
    fn extract_takes_last_block_when_agent_restates_protocol() {
        let reply = "协议要求：\n```completion-output\n<示例内容>\n```\n实际交付：\n```completion-output\n真实交付说明\n```\n";
        assert_eq!(extract_completion_output(reply), Some("真实交付说明\n"));
    }

    #[test]
    fn extract_accepts_unclosed_fence_at_eof() {
        let reply = "结论如下：\n```completion-output\n部署地址: http://y";
        assert_eq!(extract_completion_output(reply), Some("部署地址: http://y"));
    }

    #[test]
    fn extract_skips_completion_output_inside_unrelated_fence() {
        // 非 completion-output 围栏内的文本是围栏内容，不当成开栏
        let reply = "```text\n```completion-output\n```\n结尾";
        assert_eq!(extract_completion_output(reply), None);
    }

    #[test]
    fn completion_output_from_reply_trims_and_blank_is_none() {
        assert_eq!(
            completion_output_from_reply("```completion-output\n  \n```\n"),
            None
        );
        assert_eq!(
            completion_output_from_reply("前文\n```completion-output\n\n  说明  \n```\n"),
            Some("说明".to_string())
        );
    }

    #[test]
    fn completion_output_caps_at_16k_chars_by_unicode_scalar() {
        // 16_001 个 CJK 字符：按 rune 截断到 16_000（服务端同口径收紧上限，非 UTF-16/字节），
        // 且尾部追加截断标记（标记计入上限——发送值恒 ≤16_000，规避 server 400 任务不置 completed）。
        let body = "门".repeat(16_001);
        let reply = format!("```completion-output\n{body}\n```\n");
        let output = completion_output_from_reply(&reply).expect("hit");
        assert_eq!(output.chars().count(), MAX_COMPLETION_OUTPUT_CHARS);
        assert!(output.ends_with(COMPLETION_OUTPUT_TRUNCATION_MARKER));
        let kept = MAX_COMPLETION_OUTPUT_CHARS - COMPLETION_OUTPUT_TRUNCATION_MARKER.chars().count();
        assert!(output.chars().take(kept).all(|c| c == '门'));
    }

    #[test]
    fn workflow_completion_output_prefers_fenced_block_over_full_reply() {
        // 块优先：收尾节点按协议产出块 → 只传块内容，不混入回复全文。
        let reply = "前置说明。\n```completion-output\n交付说明\n```\n收尾致意。";
        assert_eq!(
            workflow_completion_output(reply),
            Some("交付说明".to_string())
        );
    }

    #[test]
    fn workflow_completion_output_falls_back_to_full_reply_on_near_miss() {
        // §12.55 事故形态（info 串另起一行，提取 fail-closed）+ 工作流 fail-open：块提不出 →
        // 最终回复全文兜底——块内容原样嵌在全文里送达 multica，交付不落空。
        let reply =
            "```\ncompletion-output\n任务性质：测试性工作项，无实质代码变更。\n当前分支：sit\n```";
        assert_eq!(workflow_completion_output(reply), Some(reply.trim().to_string()));
    }

    #[test]
    fn workflow_completion_output_full_reply_fallback_truncates_with_marker() {
        // 兜底全文同样受 16k 预截断约束（超长标记计内）。
        let reply = format!("无块的超长回复：{}", "长".repeat(20_000));
        let output = workflow_completion_output(&reply).expect("fallback");
        assert_eq!(output.chars().count(), MAX_COMPLETION_OUTPUT_CHARS);
        assert!(output.ends_with(COMPLETION_OUTPUT_TRUNCATION_MARKER));
    }

    #[test]
    fn workflow_completion_output_blank_reply_is_none() {
        // 全文兜底后仍无内容（空/纯空白回复）→ None：键缺席，不动 issue 现值。
        assert_eq!(workflow_completion_output("  \n "), None);
        assert_eq!(workflow_completion_output(""), None);
    }

    #[test]
    fn probe_flags_info_string_written_on_own_line_after_bare_fence() {
        // §12.55 内网实测断裂形态：agent 输出裸围栏 + info 串另起一行。GFM 语义下不是
        // completion-output 块——提取必须继续返回 None（fail-closed），但探测要给出可检索
        // 原因（门控日志升级 WARN，内网不再需要导出 timeline 取证）。
        let reply =
            "```\ncompletion-output\n任务性质：测试性工作项，无实质代码变更。\n当前分支：sit\n```";
        let probe = completion_block_probe(reply);
        assert_eq!(probe.block, None);
        assert_eq!(probe.near_miss_reason, Some(NEAR_MISS_INFO_ON_OWN_LINE));
    }

    #[test]
    fn probe_flags_fence_info_string_similar_but_not_exact() {
        let probe = completion_block_probe("```completion-output:\n交付说明\n```");
        assert_eq!(probe.block, None);
        assert_eq!(probe.near_miss_reason, Some(NEAR_MISS_SIMILAR_INFO_STRING));
    }

    #[test]
    fn probe_stays_quiet_for_prose_mention_without_fence_slip() {
        // agent 按协议解释「未完成不得输出块」（prose 提及 info 串）→ 非近似形态，不告警：
        // 正常扣留轮次不得被误报成格式错误（观测 timeline 第 2/3 轮均为此类回复）。
        let probe = completion_block_probe("工作尚未完成，等确认后再附加 completion-output 块。");
        assert_eq!(probe.block, None);
        assert_eq!(probe.near_miss_reason, None);
    }

    #[test]
    fn probe_hits_block_without_near_miss() {
        let probe = completion_block_probe("完成。\n```completion-output\n交付说明\n```\n");
        assert_eq!(probe.block, Some("交付说明".to_string()));
        assert_eq!(probe.near_miss_reason, None);
    }

    #[test]
    fn final_assistant_reply_reads_last_non_placeholder_text_delta() {
        let temp = tempfile::tempdir().expect("tempdir");
        let attempt_dir =
            Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).expect("utf8 temp path");
        let paths = AcpAttemptPaths::from_attempt_dir(attempt_dir.clone());
        gold_band::acp::events::write_timeline_items(
            &paths.timeline,
            &[
                text_delta("user-echo", 1, "用户输入"),
                text_delta("assistant-1", 2, "中间说明"),
                text_delta(
                    "assistant-final",
                    3,
                    "最终回复\n```completion-output\n交付\n```\n",
                ),
            ],
        )
        .expect("write timeline");

        assert_eq!(
            final_assistant_reply(attempt_dir.as_str()),
            Some("最终回复\n```completion-output\n交付\n```\n".to_string())
        );
        assert_eq!(
            completion_output_from_reply(
                final_assistant_reply(attempt_dir.as_str())
                    .as_deref()
                    .unwrap_or_default()
            ),
            Some("交付".to_string())
        );
    }

    #[test]
    fn final_assistant_reply_missing_dir_is_none() {
        let missing =
            Utf8PathBuf::from_path_buf(std::env::temp_dir().join("gold-band-handoff-missing"))
                .expect("utf8 temp path");
        assert_eq!(final_assistant_reply(missing.as_str()), None);
    }

    fn text_delta(id: &str, seq: u64, content: &str) -> AcpUiEvent {
        AcpUiEvent {
            id: id.to_string(),
            seq,
            timestamp: format!("{seq}Z"),
            kind: "textDelta".to_string(),
            session_id: None,
            content: Some(content.to_string()),
            title: None,
            tool_call_id: None,
            status: None,
            started_seq: Some(seq),
            ended_seq: Some(seq),
            started_at: Some(format!("{seq}Z")),
            ended_at: Some(format!("{seq}Z")),
            timing: None,
            raw: None,
        }
    }
}
