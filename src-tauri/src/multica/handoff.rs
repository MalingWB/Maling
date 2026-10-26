//! multica issue 完成输出（completion_output）提取（issue 完成输出传递特性，码灵侧设计方案 §5）。
//!
//! 写路径协议：agent 产出 info 为 `completion-output` 的代码围栏块；run 成功收尾时块作为
//! `completion_output` 随 `POST /complete` 同事务上送（multica C6 改造，2026-10：完成与写输出
//! 单请求、后写胜，取代此前 `update_issue_status(done)` PUT 附带通道）。direct 模式由该块**门控**
//! 完成（缺块扣留，见 bridge，口径=最后一条 assistant 消息）；工作流收尾节点带 JSON 输出验证时
//! 最后一条消息被裁决 JSON 占用、块在它之前的人类可读消息里，故工作流口径按
//! [`attempt_completion_scan`] 在**全部** assistant 消息中倒序找块（last-block-wins）——只认块，
//! 提不出则不发送该字段（multica 保留 issue 现值），不做全文兜底（机器裁决/报告全文都不是交付物）。

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

/// attempt 完成扫描结果（一次 timeline 读，两个口径的块探测，零额外 I/O）。
///
/// - [`AttemptCompletionScan::final_probe`]：**最后一条** assistant 消息——direct 门控（run/turn
///   收尾）与终态后补交的协议口径，块必须出现在「最终回复末尾」。
/// - [`AttemptCompletionScan::attempt_probe`]：**全部** assistant 消息倒序找块（last-block-wins，
///   与 multica「多次输出整体覆盖」一致）——工作流收尾采集口径：收尾节点带 JSON 输出验证
///   （默认 accept `{"reason","result"}`）时最后一条消息被裁决 JSON 占用，块在它之前的人类
///   可读消息里（M5-cd 内网实测：只看最后一条 → 裁决 JSON 被当交付物上送）。
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct AttemptCompletionScan {
    pub(crate) final_probe: CompletionBlockProbe,
    pub(crate) attempt_probe: CompletionBlockProbe,
}

/// 工作流收尾采集入口（run 成功收尾，multica C6 改造清单 §2.1）：attempt 级找块。
///
/// 块命中 → `attempt_probe.block` 即上送值（已 trim + 16k 预截断）；提不出块（未产出/近似形态
/// 滑落）→ `block` None，调用方**不发送该字段**（multica 保留 issue 现值），`near_miss_reason`
/// 给出机器可检索原因（观测日志用）。读失败 → 全空 probe（fail-open，issue 照常流转）。
pub(crate) fn attempt_completion_scan(attempt_dir: &str) -> AttemptCompletionScan {
    let Some(items) = assistant_text_items(attempt_dir) else {
        return AttemptCompletionScan::default();
    };
    // 借用指针视图（无内容复制）：final 口径取末位，attempt 口径倒序含末位全量。
    let texts: Vec<&str> = items
        .iter()
        .filter(|item| is_assistant_text(item))
        .filter_map(|item| item.content.as_deref())
        .collect();
    let final_probe = texts
        .last()
        .map(|reply| completion_block_probe(reply))
        .unwrap_or_default();
    let mut attempt_probe = CompletionBlockProbe::default();
    for reply in texts.iter().rev() {
        let probe = completion_block_probe(reply);
        if probe.block.is_some() {
            attempt_probe = probe;
            break;
        }
        // 无块时保留**最新**一条近似形态原因（倒序首遇），诊断最贴近终态。
        if attempt_probe.near_miss_reason.is_none() {
            attempt_probe.near_miss_reason = probe.near_miss_reason;
        }
    }
    AttemptCompletionScan {
        final_probe,
        attempt_probe,
    }
}

/// 最后一条 assistant 消息的块探测（direct turn 门控与终态后补交口径，单次 timeline 读）。
pub(crate) fn final_completion_probe(attempt_dir: &str) -> CompletionBlockProbe {
    let Some(items) = assistant_text_items(attempt_dir) else {
        return CompletionBlockProbe::default();
    };
    items
        .iter()
        .filter(|item| is_assistant_text(item))
        .next_back()
        .and_then(|item| item.content.as_deref())
        .map(completion_block_probe)
        .unwrap_or_default()
}

/// attempt timeline 物化（读失败 → None 并 WARN，fail-open）。调用方直接借用 items 探测，
/// 不复制消息内容——补交通道逐 turn 触发，避免整份文本的无谓克隆。
fn assistant_text_items(attempt_dir: &str) -> Option<Vec<AcpUiEvent>> {
    let paths = AcpAttemptPaths::from_attempt_dir(attempt_dir.into());
    match load_timeline_items(&paths.timeline) {
        Ok(items) => Some(items),
        Err(error) => {
            tracing::warn!(
                timeline = %paths.timeline,
                %error,
                "multica completion-output: timeline read failed (fail-open)"
            );
            None
        }
    }
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
        let kept =
            MAX_COMPLETION_OUTPUT_CHARS - COMPLETION_OUTPUT_TRUNCATION_MARKER.chars().count();
        assert!(output.chars().take(kept).all(|c| c == '门'));
    }

    #[test]
    fn attempt_scan_delivers_block_from_earlier_message_when_last_is_verdict_json() {
        // M5-cd 内网实测回归（2026-10-23）：accept 节点带 JSON 输出验证（`{"reason","result"}`），
        // 最后一条 assistant 消息是裁决 JSON、completion-output 块在它之前的主回复里。工作流
        // 采集（attempt 口径）必须交付块；final 口径（direct 门控/补交，块须在最终回复末尾）
        // 对裁决 JSON 收尾的形态不命中——两口径的差异即本回归的固化点。
        let (attempt_dir, _temp) = attempt_dir_with_texts(&[
            "验证报告。\n```completion-output\n本次交付：任务重试链路测试通过。\n```\n",
            "{\"reason\":\"验收完成\",\"result\":true}",
        ]);
        let scan = attempt_completion_scan(attempt_dir.as_str());
        assert_eq!(
            scan.attempt_probe.block,
            Some("本次交付：任务重试链路测试通过。".to_string())
        );
        assert_eq!(scan.attempt_probe.near_miss_reason, None);
        assert_eq!(scan.final_probe.block, None);
    }

    #[test]
    fn attempt_scan_takes_last_block_across_messages() {
        // 多条消息各含块 → last-block-wins（multica「多次输出整体覆盖，只保留最终结论」）。
        let (attempt_dir, _temp) = attempt_dir_with_texts(&[
            "```\ncompletion-output\n草稿块，应被覆盖\n```\n",
            "结论定稿。\n```completion-output\n最终交付说明\n```\n",
        ]);
        let scan = attempt_completion_scan(attempt_dir.as_str());
        assert_eq!(scan.attempt_probe.block, Some("最终交付说明".to_string()));
        assert_eq!(scan.final_probe.block, Some("最终交付说明".to_string()));
    }

    #[test]
    fn attempt_scan_near_miss_in_earlier_message_is_flagged_not_delivered() {
        // §12.55 形态滑落在前置消息：attempt 口径不发送字段（near-miss 原因可检索），裁决 JSON
        // 收尾的最后一条消息不得成为交付内容。
        let (attempt_dir, _temp) = attempt_dir_with_texts(&[
            "```\ncompletion-output\n任务性质：测试性工作项，无实质代码变更。\n```",
            "{\"reason\":\"验收完成\",\"result\":true}",
        ]);
        let scan = attempt_completion_scan(attempt_dir.as_str());
        assert_eq!(scan.attempt_probe.block, None);
        assert_eq!(
            scan.attempt_probe.near_miss_reason,
            Some(NEAR_MISS_INFO_ON_OWN_LINE)
        );
    }

    #[test]
    fn attempt_scan_without_block_or_missing_dir_is_default() {
        // 无块（正常未产出，issue 现值保留）/ attempt 缺失（非 ACP 完成路径）→ 全空 probe。
        let (attempt_dir, _temp) = attempt_dir_with_texts(&["工作完成，无交付说明块。"]);
        let scan = attempt_completion_scan(attempt_dir.as_str());
        assert_eq!(scan.final_probe.block, None);
        assert_eq!(scan.attempt_probe.block, None);
        assert_eq!(scan.attempt_probe.near_miss_reason, None);

        let missing =
            Utf8PathBuf::from_path_buf(std::env::temp_dir().join("gold-band-scan-missing"))
                .expect("utf8 temp path");
        assert_eq!(
            attempt_completion_scan(missing.as_str()),
            AttemptCompletionScan::default()
        );
    }

    #[test]
    fn final_completion_probe_only_looks_at_last_message() {
        // final 口径（direct 门控/终态后补交）只看最后一条消息：块仅在更早消息 → 不命中
        // （协议要求块在「最终回复末尾」）；块在最后一条 → 命中。
        let (earlier_only, _temp) = attempt_dir_with_texts(&[
            "```completion-output\n前置消息的块\n```\n",
            "最终致意，无块。",
        ]);
        assert_eq!(final_completion_probe(earlier_only.as_str()).block, None);

        let (last_has_block, _temp2) = attempt_dir_with_texts(&[
            "前置说明。",
            "最终回复\n```completion-output\n交付说明\n```\n",
        ]);
        assert_eq!(
            final_completion_probe(last_has_block.as_str()).block,
            Some("交付说明".to_string())
        );
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

    /// 构造含多条 assistant 文本消息（按给定顺序、seq 递增）的 attempt timeline 目录。
    /// 返回 TempDir 供测试持有（drop 即删除目录，读取须在其存活期间完成）。
    fn attempt_dir_with_texts(texts: &[&str]) -> (Utf8PathBuf, tempfile::TempDir) {
        let temp = tempfile::tempdir().expect("tempdir");
        let attempt_dir =
            Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).expect("utf8 temp path");
        let items: Vec<AcpUiEvent> = texts
            .iter()
            .enumerate()
            .map(|(index, text)| text_delta(&format!("assistant-{index}"), index as u64 + 1, text))
            .collect();
        let paths = AcpAttemptPaths::from_attempt_dir(attempt_dir.clone());
        gold_band::acp::events::write_timeline_items(&paths.timeline, &items)
            .expect("write timeline");
        (attempt_dir, temp)
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
