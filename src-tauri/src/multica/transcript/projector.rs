//! `AcpUiEvent` → multica 执行记录消息 的纯投影。
//!
//! 观测点在 `acp_live_update_emitter`（IPC 压缩前），因此这里拿得到完整 `raw`
//! （`rawInput` / `rawOutput` / agent transcript 元数据）。live 文本事件是 **全量累积快照**
//! （同一个 stable item id 反复携带全量 content），所以文本按「已见过的字符数」做差量投影，
//! 而不是把每次快照整段重发。
//!
//! 投影只产出「消息」，seq 由 reporter 取批时统一分配，保证 seq 顺序 = 事件顺序。

use std::collections::{HashMap, VecDeque};

use gold_band::acp::events::{
    AcpUiEvent, agent_transcript_tool_output, extract_agent_transcript_relation,
};
use serde_json::{Value, json};

use super::model::{
    MAX_BATCH_BYTES, MAX_BATCH_MESSAGES, MAX_OUTBOX_MESSAGES, MAX_TEXT_CHARS,
    MAX_TOOL_OUTPUT_CHARS, MAX_TRACKED_STREAMS, MAX_TRACKED_TOOLS, REPORT_THINKING, TYPE_TEXT,
    TYPE_THINKING, TranscriptMessage, sanitize_text, truncate_chars,
};
use super::tool_input::normalize_tool_input;

/// 工具行已到达终态状态集合（与 Gold Band 前端 `isTerminalToolStatus` 对齐）。
const TERMINAL_TOOL_STATUSES: [&str; 8] = [
    "completed",
    "success",
    "succeeded",
    "failed",
    "error",
    "cancelled",
    "canceled",
    "rejected",
];

struct Segment {
    kind: &'static str,
    item_id: String,
    content: String,
}

struct ToolState {
    name: String,
    use_sent: bool,
    result_sent: bool,
}

pub(crate) struct Projector {
    outbox: VecDeque<TranscriptMessage>,
    segment: Option<Segment>,
    /// stable item id → 已投影的字符数（live 文本是全量快照）。
    streams: HashMap<String, usize>,
    stream_order: VecDeque<String>,
    /// tool_call_id → 工具行状态（`use` 只能发一次，`result` 只能发一次）。
    tools: HashMap<String, ToolState>,
    tool_order: VecDeque<String>,
    dropped_messages: u64,
}

impl Projector {
    pub(crate) fn new() -> Self {
        Self {
            outbox: VecDeque::new(),
            segment: None,
            streams: HashMap::new(),
            stream_order: VecDeque::new(),
            tools: HashMap::new(),
            tool_order: VecDeque::new(),
            dropped_messages: 0,
        }
    }

    /// 观测一个 live 事件。非上报类型由调用方过滤，这里再兜一层。
    pub(crate) fn observe(&mut self, event: &AcpUiEvent) {
        match event.kind.as_str() {
            "textDelta" => self.observe_text(event, TYPE_TEXT),
            "thoughtDelta" if REPORT_THINKING => self.observe_text(event, TYPE_THINKING),
            "toolCall" | "toolCallUpdate" => self.observe_tool(event),
            _ => {}
        }
    }

    /// 运行期错误（失败/取消）作为一条 `error` 行进入消息流。
    pub(crate) fn push_error(&mut self, message: &str) {
        let content = truncate_chars(&sanitize_text(message), MAX_TEXT_CHARS);
        if content.trim().is_empty() {
            return;
        }
        self.close_segment();
        self.push(TranscriptMessage::error(content));
    }

    /// 收起当前文本分段。每次 flush 前调用，让文本按 flush 节奏成为独立消息，
    /// 而不是整个回合攒成一条巨块。
    pub(crate) fn close_segment(&mut self) {
        let Some(segment) = self.segment.take() else {
            return;
        };
        if segment.content.trim().is_empty() {
            return;
        }
        self.push(TranscriptMessage::body(segment.kind, segment.content));
    }

    /// 取走一批待上报消息并分配 seq（顺序 = 事件顺序）。
    ///
    /// 按条数与序列化字节数双上限切批：服务端没有限制，但单请求体过大会放大失败代价
    /// （批次原子，一失败整批丢）；剩下的留在缓冲里由下一拍继续发。
    pub(crate) fn drain(&mut self, next_seq: &mut u64) -> Vec<TranscriptMessage> {
        let mut batch: Vec<TranscriptMessage> = Vec::new();
        let mut bytes = 0usize;
        while batch.len() < MAX_BATCH_MESSAGES {
            let Some(front) = self.outbox.front() else {
                break;
            };
            let size = serde_json::to_vec(front)
                .map(|body| body.len())
                .unwrap_or(0);
            if !batch.is_empty() && bytes.saturating_add(size) > MAX_BATCH_BYTES {
                break;
            }
            let Some(mut message) = self.outbox.pop_front() else {
                break;
            };
            message.seq = *next_seq;
            *next_seq = next_seq.saturating_add(1);
            bytes = bytes.saturating_add(size);
            batch.push(message);
        }
        batch
    }

    pub(crate) fn dropped_messages(&self) -> u64 {
        self.dropped_messages
    }

    fn observe_text(&mut self, event: &AcpUiEvent, kind: &'static str) {
        let Some(content) = event.content.as_deref() else {
            return;
        };
        if content.is_empty() {
            return;
        }
        let content = sanitize_text(content);
        let total = content.chars().count();
        let seen = self.streams.get(&event.id).copied().unwrap_or(0);
        // 常规情形 live 文本是全量快照 → 只取新增部分；若快照变短（压缩/重建），
        // 说明旧快照已失效，按新分段整段重投影，避免静默丢内容。
        let delta = if total >= seen {
            content.chars().skip(seen).collect::<String>()
        } else {
            content.clone()
        };
        self.remember_stream(&event.id, total);
        if delta.is_empty() {
            return;
        }

        let mut pending = delta;
        loop {
            let need_new_segment = self.segment.as_ref().is_none_or(|segment| {
                segment.kind != kind
                    || segment.item_id != event.id
                    || segment.content.chars().count() >= MAX_TEXT_CHARS
            });
            if need_new_segment {
                self.close_segment();
                self.segment = Some(Segment {
                    kind,
                    item_id: event.id.clone(),
                    content: String::new(),
                });
            }
            let Some(segment) = self.segment.as_mut() else {
                return;
            };
            let used = segment.content.chars().count();
            let take = (MAX_TEXT_CHARS - used).min(pending.chars().count());
            let split_at = pending
                .char_indices()
                .nth(take)
                .map(|(index, _)| index)
                .unwrap_or(pending.len());
            segment.content.push_str(&pending[..split_at]);
            if split_at >= pending.len() {
                return;
            }
            pending = pending[split_at..].to_string();
        }
    }

    fn observe_tool(&mut self, event: &AcpUiEvent) {
        let key = event
            .tool_call_id
            .clone()
            .unwrap_or_else(|| event.id.clone());
        let name = match self.tools.get(&key) {
            Some(state) => state.name.clone(),
            None => {
                let name = agent_tool_name(event);
                self.remember_tool(&key, name.clone());
                name
            }
        };
        let terminal = event.status.as_deref().is_some_and(is_terminal_tool_status);

        let use_sent = self.tools.get(&key).is_some_and(|state| state.use_sent);
        if !use_sent {
            if let Some(input) = event_tool_input(event) {
                self.mark_tool(&key, |state| state.use_sent = true);
                self.push(TranscriptMessage::tool_use(
                    name.clone(),
                    normalize_tool_input(input),
                ));
            } else if terminal {
                self.mark_tool(&key, |state| state.use_sent = true);
                self.push(TranscriptMessage::tool_use(name.clone(), json!({})));
            }
        }

        let result_sent = self.tools.get(&key).is_some_and(|state| state.result_sent);
        if terminal && !result_sent {
            // 结果必须排在 use 之后：没有输入也要补一条 use，保证前端能配对。
            if !self.tools.get(&key).is_some_and(|state| state.use_sent) {
                self.mark_tool(&key, |state| state.use_sent = true);
                self.push(TranscriptMessage::tool_use(name.clone(), json!({})));
            }
            self.mark_tool(&key, |state| state.result_sent = true);
            let output = event_tool_output(event)
                .map(|output| truncate_chars(&sanitize_text(&output), MAX_TOOL_OUTPUT_CHARS))
                .unwrap_or_default();
            self.push(TranscriptMessage::tool_result(name, output));
        }
    }

    fn mark_tool(&mut self, key: &str, update: impl FnOnce(&mut ToolState)) {
        if let Some(state) = self.tools.get_mut(key) {
            update(state);
        }
    }

    fn remember_stream(&mut self, item_id: &str, total: usize) {
        if self.streams.contains_key(item_id) {
            self.streams.insert(item_id.to_string(), total);
            return;
        }
        if self.streams.len() >= MAX_TRACKED_STREAMS
            && let Some(evicted) = self.stream_order.pop_front()
        {
            self.streams.remove(&evicted);
        }
        self.streams.insert(item_id.to_string(), total);
        self.stream_order.push_back(item_id.to_string());
    }

    fn remember_tool(&mut self, key: &str, name: String) {
        if self.tools.contains_key(key) {
            return;
        }
        if self.tools.len() >= MAX_TRACKED_TOOLS
            && let Some(evicted) = self.tool_order.pop_front()
        {
            self.tools.remove(&evicted);
        }
        self.tools.insert(
            key.to_string(),
            ToolState {
                name,
                use_sent: false,
                result_sent: false,
            },
        );
        self.tool_order.push_back(key.to_string());
    }

    fn push(&mut self, message: TranscriptMessage) {
        if self.outbox.len() >= MAX_OUTBOX_MESSAGES {
            // 溢出时优先丢最旧的文本行，工具行（含 use/result 配对）尽量保留。
            let evict = self
                .outbox
                .iter()
                .position(|item| matches!(item.message_type, TYPE_TEXT | TYPE_THINKING))
                .unwrap_or(0);
            self.outbox.remove(evict);
            self.dropped_messages = self.dropped_messages.saturating_add(1);
        }
        self.outbox.push_back(message);
    }
}

fn is_terminal_tool_status(status: &str) -> bool {
    let status = status.trim().to_ascii_lowercase();
    TERMINAL_TOOL_STATUSES.contains(&status.as_str())
}

/// 工具名优先级与 Gold Band 前端一致，另加 agent transcript 元数据兜底：
/// `_meta.goldBandConversation.toolName` → agent transcript 关系 → 标题首词 → `tool`。
fn agent_tool_name(event: &AcpUiEvent) -> String {
    if let Some(name) = event
        .raw
        .as_ref()
        .and_then(|raw| raw.pointer("/_meta/goldBandConversation/toolName"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        return name.to_string();
    }
    if let Some(name) = event
        .raw
        .as_ref()
        .and_then(extract_agent_transcript_relation)
        .and_then(|relation| relation.tool_name)
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
    {
        return name;
    }
    if event_tool_input(event)
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)
        .is_some()
    {
        return "Bash".to_string();
    }
    if let Some(token) = event
        .title
        .as_deref()
        .and_then(|title| title.split_whitespace().next())
        .filter(|token| !token.is_empty())
    {
        return token.to_string();
    }
    "tool".to_string()
}

fn event_tool_input(event: &AcpUiEvent) -> Option<&Value> {
    let raw = event.raw.as_ref()?;
    raw.pointer("/toolCall/rawInput")
        .or_else(|| raw.pointer("/toolCall/input"))
        .or_else(|| raw.get("rawInput"))
        .or_else(|| raw.get("input"))
        .filter(|value| value.is_object())
}

fn event_tool_output(event: &AcpUiEvent) -> Option<String> {
    let raw = event.raw.as_ref()?;
    if let Some(output) = agent_transcript_tool_output(raw).and_then(value_to_text) {
        return Some(output);
    }
    for pointer in [
        "/toolCall/rawOutput/formatted_output",
        "/toolCall/output",
        "/rawOutput/formatted_output",
        "/output",
    ] {
        if let Some(output) = raw.pointer(pointer).and_then(value_to_text) {
            return Some(output);
        }
    }
    if let Some(content) = raw.pointer("/toolCall/content").and_then(value_to_text) {
        return Some(content);
    }
    None
}

/// `rawOutput` 在 ACP 里可能是字符串、`{text}`、`[{type:"text",text}]` 或
/// `{content:[{type:"text",text}]}`，统一取纯文本。
fn value_to_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => (!text.is_empty()).then(|| text.clone()),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().filter_map(value_to_text).collect();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                return (!text.is_empty()).then(|| text.to_string());
            }
            if let Some(content) = object.get("content").and_then(value_to_text) {
                return Some(content);
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, kind: &str) -> AcpUiEvent {
        AcpUiEvent {
            id: id.to_string(),
            seq: 1,
            timestamp: "2026-11-12T00:00:00Z".to_string(),
            kind: kind.to_string(),
            session_id: None,
            content: None,
            title: None,
            tool_call_id: None,
            status: None,
            started_seq: None,
            ended_seq: None,
            started_at: None,
            ended_at: None,
            timing: None,
            raw: None,
        }
    }

    fn text_chunk(id: &str, content: &str) -> AcpUiEvent {
        let mut event = event(id, "textDelta");
        event.content = Some(content.to_string());
        event
    }

    fn tool_event(id: &str, tool_call_id: &str, status: &str, input: Value) -> AcpUiEvent {
        let mut event = event(id, "toolCallUpdate");
        event.tool_call_id = Some(tool_call_id.to_string());
        event.status = Some(status.to_string());
        event.raw = Some(json!({"toolCall": {"rawInput": input}}));
        event
    }

    fn drain(projector: &mut Projector) -> Vec<TranscriptMessage> {
        let mut next_seq = 1;
        projector.drain(&mut next_seq)
    }

    #[test]
    fn live_text_snapshots_are_projected_as_increments() {
        let mut projector = Projector::new();
        projector.observe(&text_chunk("item-1", "你"));
        projector.observe(&text_chunk("item-1", "你好"));
        projector.observe(&text_chunk("item-1", "你好，世界"));
        projector.close_segment();
        let batch = drain(&mut projector);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].message_type, TYPE_TEXT);
        assert_eq!(batch[0].content.as_deref(), Some("你好，世界"));
        assert_eq!(batch[0].seq, 1);
    }

    #[test]
    fn text_segments_split_on_flush_and_item_change() {
        let mut projector = Projector::new();
        projector.observe(&text_chunk("item-1", "第一段"));
        projector.close_segment();
        projector.observe(&text_chunk("item-1", "第一段第二段"));
        projector.observe(&text_chunk("item-2", "另一个流"));
        projector.close_segment();
        let batch = drain(&mut projector);
        assert_eq!(batch.len(), 3);
        assert_eq!(batch[0].content.as_deref(), Some("第一段"));
        assert_eq!(batch[1].content.as_deref(), Some("第二段"));
        assert_eq!(batch[2].content.as_deref(), Some("另一个流"));
        assert_eq!(
            batch.iter().map(|m| m.seq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn shrinking_snapshot_reprojects_full_content() {
        let mut projector = Projector::new();
        projector.observe(&text_chunk("item-1", "abcdef"));
        projector.close_segment();
        projector.observe(&text_chunk("item-1", "abc"));
        projector.close_segment();
        let batch = drain(&mut projector);
        assert_eq!(batch[0].content.as_deref(), Some("abcdef"));
        assert_eq!(batch[1].content.as_deref(), Some("abc"));
    }

    #[test]
    fn thinking_is_skipped_when_disabled() {
        let mut projector = Projector::new();
        let mut event = event("thought-1", "thoughtDelta");
        event.content = Some("内部推理".to_string());
        projector.observe(&event);
        projector.close_segment();
        assert!(drain(&mut projector).is_empty());
    }

    #[test]
    fn tool_use_is_emitted_once_then_result_pairs_by_tool_call_id() {
        let mut projector = Projector::new();
        projector.observe(&tool_event(
            "call-1",
            "call-1",
            "in_progress",
            json!({"file_path": "a.rs", "content": "hello"}),
        ));
        projector.observe(&tool_event(
            "update-2",
            "call-1",
            "in_progress",
            json!({"file_path": "a.rs", "content": "hello"}),
        ));
        let mut done = tool_event("update-3", "call-1", "completed", json!({}));
        done.raw = Some(json!({"toolCall": {"content": [{"type": "text", "text": "ok"}]}}));
        projector.observe(&done);
        let batch = drain(&mut projector);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].message_type, super::super::model::TYPE_TOOL_USE);
        assert_eq!(batch[0].tool.as_deref(), Some("tool"));
        assert_eq!(
            batch[0].input,
            Some(json!({"file_path": "a.rs", "content": "hello"}))
        );
        assert_eq!(batch[1].message_type, super::super::model::TYPE_TOOL_RESULT);
        assert_eq!(batch[1].output.as_deref(), Some("ok"));
    }

    #[test]
    fn terminal_tool_without_input_still_emits_use_then_result() {
        let mut projector = Projector::new();
        let mut done = event("call-9", "toolCallUpdate");
        done.tool_call_id = Some("call-9".to_string());
        done.status = Some("failed".to_string());
        done.raw = Some(json!({"output": "boom"}));
        projector.observe(&done);
        let batch = drain(&mut projector);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].message_type, super::super::model::TYPE_TOOL_USE);
        assert_eq!(batch[0].input, Some(json!({})));
        assert_eq!(batch[1].output.as_deref(), Some("boom"));
    }

    #[test]
    fn tool_output_is_capped() {
        let mut projector = Projector::new();
        let mut done = tool_event("call-1", "call-1", "completed", json!({"command": "x"}));
        done.raw = Some(json!({"output": "y".repeat(MAX_TOOL_OUTPUT_CHARS * 2)}));
        projector.observe(&done);
        let batch = drain(&mut projector);
        assert_eq!(
            batch[1].output.as_deref().map(str::len),
            Some(MAX_TOOL_OUTPUT_CHARS)
        );
    }

    #[test]
    fn outbox_overflow_drops_oldest_text_first() {
        let mut projector = Projector::new();
        for index in 0..(MAX_OUTBOX_MESSAGES + 10) {
            projector.push(TranscriptMessage::body(TYPE_TEXT, format!("t{index}")));
        }
        assert_eq!(projector.dropped_messages(), 10);
        // 溢出丢最旧文本后仍有 MAX_OUTBOX_MESSAGES 条；单拍按上限切批，剩余下一拍补发。
        let mut next_seq = 1;
        let mut total = 0usize;
        let mut first = true;
        loop {
            let batch = projector.drain(&mut next_seq);
            if batch.is_empty() {
                break;
            }
            if first {
                // 首拍恰好卡在条数上限，且第一条已是被丢剩下的 t10。
                assert_eq!(batch.len(), MAX_BATCH_MESSAGES);
                assert_eq!(batch[0].content.as_deref(), Some("t10"));
                first = false;
            }
            // seq 跨批连续，不重号。
            assert_eq!(batch[0].seq, total as u64 + 1);
            total += batch.len();
        }
        assert_eq!(total, MAX_OUTBOX_MESSAGES);
    }

    #[test]
    fn error_line_is_sanitized_and_capped() {
        let mut projector = Projector::new();
        projector.push_error("boom\0");
        let batch = drain(&mut projector);
        assert_eq!(batch[0].message_type, super::super::model::TYPE_ERROR);
        assert_eq!(batch[0].content.as_deref(), Some("boom"));
    }
}
