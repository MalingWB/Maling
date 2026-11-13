//! 工具输入归一化：把各家 Agent 的 `rawInput` 收敛成 multica 前端认识的三种形状。
//!
//! multica 的 `tool_use.input` 约定（其 daemon 同款）：
//! ① 文件变更：`{"changes":[{"path","kind":"add|update|delete|move","diff","move_path"?}]}`
//! ② 整文件写：`{"file_path","content"}`
//! ③ 精确编辑：`{"file_path","old_string","new_string"}`
//! 另外 shell 类工具用 `input.command`。
//!
//! 归一化是 **形状优先** 的纯函数：先看有没有现成的 ①②③ / command 形状，再退到 Codex
//! `apply_patch` 文本解析；都不命中就把原对象按上限透传（multica 前端会 generic 渲染）。
//! 绝不依赖工具名，避免 agent / provider 差异造成漏报。

use serde_json::{Map, Value, json};

use super::model::{MAX_INPUT_BYTES, MAX_INPUT_FIELD_CHARS, sanitize_text, truncate_chars};

const PATCH_MARKERS: [&str; 4] = [
    "*** Begin Patch",
    "*** Update File:",
    "*** Add File:",
    "*** Delete File:",
];

/// 归一化入口：必定返回一个 JSON 对象（形状优先，兜底透传 + 上限收紧）。
pub(crate) fn normalize_tool_input(input: &Value) -> Value {
    let Some(object) = input.as_object() else {
        return json!({});
    };
    let normalized = normalize_object(object);
    cap_serialized(normalized)
}

fn normalize_object(object: &Map<String, Value>) -> Value {
    if let Some(changes) = object.get("changes").and_then(Value::as_array) {
        let changes: Vec<Value> = changes.iter().filter_map(change_object).collect();
        if !changes.is_empty() {
            return json!({ "changes": changes });
        }
    }

    if let Some(file_path) = first_string(object, &["file_path", "path", "file"]) {
        if let Some(content) = object.get("content").and_then(Value::as_str) {
            return json!({
                "file_path": file_path,
                "content": cap_field(content),
            });
        }
        if let (Some(old_string), Some(new_string)) = (
            object.get("old_string").and_then(Value::as_str),
            object.get("new_string").and_then(Value::as_str),
        ) {
            return json!({
                "file_path": file_path,
                "old_string": cap_field(old_string),
                "new_string": cap_field(new_string),
            });
        }
        if let Some(edits) = object.get("edits").and_then(Value::as_array)
            && let Some(synthesized) = edits_to_changes(&file_path, edits)
        {
            return json!({ "changes": synthesized });
        }
    }

    if let Some(changes) = object
        .iter()
        .find_map(|(key, value)| patch_candidate(key, value))
        .and_then(parse_patch)
    {
        return json!({ "changes": changes });
    }

    if let Some(command) = object.get("command").and_then(Value::as_str) {
        return json!({ "command": cap_field(command) });
    }

    Value::Object(object.clone())
}

fn first_string(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

/// 已经带 `changes` 数组时，把每项收敛成 `{path, kind, diff, move_path?}`。
fn change_object(item: &Value) -> Option<Value> {
    let object = item.as_object()?;
    let path = first_string(object, &["path", "file_path", "file"])?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| matches!(value.as_str(), "add" | "update" | "delete" | "move"))
        .unwrap_or_else(|| "update".to_string());
    let mut change = Map::new();
    change.insert("path".into(), Value::String(path));
    change.insert("kind".into(), Value::String(kind));
    if let Some(diff) = object.get("diff").and_then(Value::as_str) {
        change.insert("diff".into(), Value::String(cap_field(diff)));
    }
    if let Some(move_path) = first_string(object, &["move_path", "move_to", "moveTo"]) {
        change.insert("move_path".into(), Value::String(move_path));
    }
    Some(Value::Object(change))
}

/// `{file_path, edits:[{old_string,new_string}...]}` → 单条 update change 的 unified diff。
fn edits_to_changes(file_path: &str, edits: &[Value]) -> Option<Vec<Value>> {
    let mut diff = String::new();
    let mut count = 0usize;
    for edit in edits {
        let Some(edit) = edit.as_object() else {
            continue;
        };
        let old_string = edit.get("old_string").and_then(Value::as_str).unwrap_or("");
        let new_string = edit.get("new_string").and_then(Value::as_str).unwrap_or("");
        if old_string.is_empty() && new_string.is_empty() {
            continue;
        }
        diff.push_str("@@\n");
        for line in old_string.lines() {
            diff.push('-');
            diff.push_str(line);
            diff.push('\n');
        }
        for line in new_string.lines() {
            diff.push('+');
            diff.push_str(line);
            diff.push('\n');
        }
        count += 1;
    }
    (count > 0).then(|| {
        vec![json!({
            "path": file_path,
            "kind": "update",
            "diff": cap_field(&diff),
        })]
    })
}

/// Codex `apply_patch`：`*** Begin Patch` / `*** Update File:` / `*** Add File:` /
/// `*** Delete File:` / `*** Move to:` / `*** End Patch`。
///
/// 兼容多种承载 key（`patch` / `input` / `diff` / `content` / `text` / `rawInput`），
/// 只对含补丁标记的字符串做解析，避免把普通文件内容误判成补丁。
fn patch_candidate<'a>(key: &str, value: &'a Value) -> Option<&'a str> {
    const KEYS: [&str; 6] = ["patch", "input", "diff", "content", "text", "rawInput"];
    if !KEYS.contains(&key) {
        return None;
    }
    let text = value.as_str()?;
    PATCH_MARKERS
        .iter()
        .any(|marker| text.contains(marker))
        .then_some(text)
}

fn parse_patch(text: &str) -> Option<Vec<Value>> {
    let mut changes: Vec<Value> = Vec::new();
    let mut current: Option<PatchChange> = None;
    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');
        if let Some(path) = line.strip_prefix("*** Update File:") {
            flush_change(&mut current, &mut changes);
            current = Some(PatchChange::new(path, "update"));
        } else if let Some(path) = line.strip_prefix("*** Add File:") {
            flush_change(&mut current, &mut changes);
            current = Some(PatchChange::new(path, "add"));
        } else if let Some(path) = line.strip_prefix("*** Delete File:") {
            flush_change(&mut current, &mut changes);
            current = Some(PatchChange::new(path, "delete"));
        } else if let Some(path) = line.strip_prefix("*** Move to:") {
            if let Some(change) = current.as_mut() {
                change.move_path = Some(path.trim().to_string());
            }
        } else if let Some(change) = current.as_mut()
            && !line.starts_with("*** Begin Patch")
            && !line.starts_with("*** End Patch")
        {
            change.diff.push_str(line);
            change.diff.push('\n');
        }
    }
    flush_change(&mut current, &mut changes);
    (!changes.is_empty()).then_some(changes)
}

struct PatchChange {
    path: String,
    kind: &'static str,
    move_path: Option<String>,
    diff: String,
}

impl PatchChange {
    fn new(path: &str, kind: &'static str) -> Self {
        Self {
            path: path.trim().to_string(),
            kind,
            move_path: None,
            diff: String::new(),
        }
    }
}

fn flush_change(current: &mut Option<PatchChange>, changes: &mut Vec<Value>) {
    let Some(change) = current.take() else {
        return;
    };
    if change.path.is_empty() {
        return;
    }
    let mut value = Map::new();
    value.insert("path".into(), Value::String(change.path));
    value.insert("kind".into(), Value::String(change.kind.to_string()));
    if !change.diff.trim().is_empty() {
        value.insert("diff".into(), Value::String(cap_field(&change.diff)));
    }
    if let Some(move_path) = change.move_path {
        value.insert("move_path".into(), Value::String(move_path));
    }
    changes.push(Value::Object(value));
}

fn cap_field(value: &str) -> String {
    truncate_chars(&sanitize_text(value), MAX_INPUT_FIELD_CHARS)
}

/// 单字段上限之后仍超整体上限时，二次收紧所有字符串；还不行就只报一个截断标记，
/// 保证批次能被服务端接受（服务端对超大体量不做拒绝，但我们不把几百 KB 塞进一条消息）。
fn cap_serialized(value: Value) -> Value {
    if serialized_len(&value) <= MAX_INPUT_BYTES {
        return value;
    }
    let tightened = cap_strings(value, MAX_INPUT_FIELD_CHARS / 8);
    if serialized_len(&tightened) <= MAX_INPUT_BYTES {
        return tightened;
    }
    json!({ "truncated": true })
}

fn serialized_len(value: &Value) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn cap_strings(value: Value, max_chars: usize) -> Value {
    match value {
        Value::String(text) => Value::String(truncate_chars(&text, max_chars)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| cap_strings(item, max_chars))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, item)| (key, cap_strings(item, max_chars)))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(value: &Value) -> &Vec<Value> {
        value
            .get("changes")
            .and_then(Value::as_array)
            .expect("changes array")
    }

    #[test]
    fn keeps_canonical_changes_shape() {
        let input = json!({
            "changes": [
                {"path": "a.rs", "kind": "update", "diff": "@@\n-a\n+b"},
                {"file_path": "b.rs", "kind": "add", "diff": "+new"},
            ]
        });
        let normalized = normalize_tool_input(&input);
        let changes = changes(&normalized);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0]["path"], "a.rs");
        assert_eq!(changes[0]["kind"], "update");
        assert_eq!(changes[1]["path"], "b.rs");
        assert_eq!(changes[1]["kind"], "add");
    }

    #[test]
    fn normalizes_whole_file_write() {
        let input = json!({"file_path": "src/a.rs", "content": "hello"});
        assert_eq!(
            normalize_tool_input(&input),
            json!({"file_path": "src/a.rs", "content": "hello"})
        );
    }

    #[test]
    fn normalizes_string_replace_edit() {
        let input = json!({"file_path": "src/a.rs", "old_string": "old", "new_string": "new"});
        assert_eq!(
            normalize_tool_input(&input),
            json!({"file_path": "src/a.rs", "old_string": "old", "new_string": "new"})
        );
    }

    #[test]
    fn synthesizes_changes_from_edits_array() {
        let input = json!({
            "file_path": "src/a.rs",
            "edits": [{"old_string": "old\nline", "new_string": "new\nline"}],
        });
        let normalized = normalize_tool_input(&input);
        let changes = changes(&normalized);
        assert_eq!(changes[0]["path"], "src/a.rs");
        assert_eq!(changes[0]["kind"], "update");
        let diff = changes[0]["diff"].as_str().unwrap();
        assert!(diff.contains("-old"), "{diff}");
        assert!(diff.contains("-line"), "{diff}");
        assert!(diff.contains("+new"), "{diff}");
    }

    #[test]
    fn parses_codex_apply_patch_text() {
        let patch = "*** Begin Patch\n\
                     *** Update File: src/a.rs\n\
                     @@ fn main\n\
                     -let a = 1;\n\
                     +let a = 2;\n\
                     *** Add File: src/b.rs\n\
                     +pub fn b() {}\n\
                     *** Delete File: src/c.rs\n\
                     *** End Patch";
        let input = json!({"input": patch});
        let normalized = normalize_tool_input(&input);
        let changes = changes(&normalized);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0]["path"], "src/a.rs");
        assert_eq!(changes[0]["kind"], "update");
        assert!(
            changes[0]["diff"].as_str().unwrap().contains("+let a = 2;"),
            "{:?}",
            changes[0]
        );
        assert_eq!(changes[1]["kind"], "add");
        assert_eq!(changes[2]["kind"], "delete");
    }

    #[test]
    fn parses_move_to_inside_update() {
        let patch = "*** Begin Patch\n\
                     *** Update File: old/path.rs\n\
                     *** Move to: new/path.rs\n\
                     @@\n\
                     -a\n\
                     +b\n\
                     *** End Patch";
        let normalized = normalize_tool_input(&json!({"patch": patch}));
        let changes = changes(&normalized);
        assert_eq!(changes[0]["path"], "old/path.rs");
        assert_eq!(changes[0]["move_path"], "new/path.rs");
        assert_eq!(changes[0]["kind"], "update");
    }

    #[test]
    fn plain_content_is_not_mistaken_for_patch() {
        let input = json!({"content": "no patch markers here"});
        assert_eq!(normalize_tool_input(&input), input);
    }

    #[test]
    fn shell_command_shape_wins_over_passthrough() {
        let input = json!({"command": "cargo test", "description": "run tests"});
        assert_eq!(
            normalize_tool_input(&input),
            json!({"command": "cargo test"})
        );
    }

    #[test]
    fn unknown_shape_passes_through() {
        let input = json!({"query": "gold band", "limit": 5});
        assert_eq!(normalize_tool_input(&input), input);
    }

    #[test]
    fn capped_input_never_exceeds_budget() {
        let input = json!({"content": "x".repeat(10 * MAX_INPUT_BYTES)});
        let normalized = normalize_tool_input(&input);
        assert!(serialized_len(&normalized) <= MAX_INPUT_BYTES);
    }

    #[test]
    fn nul_bytes_are_removed() {
        let input = json!({"file_path": "a.rs", "content": "a\0b"});
        assert_eq!(normalize_tool_input(&input)["content"], "ab");
    }
}
