//! multica 需求正文的 markdown 转义解码（markdown 源码 → 纯文本，开发设计 M5-bx 方案 A）。
//!
//! 根因：multica web issue 编辑器（ContentEditor，tiptap）保存走
//! `@tiptap/markdown` 序列化，正文中 `_` `*` `[` `]` `~` 等会被转义成 `\_` `\*` 存库；
//! web 端展示时再解析 markdown（`\_` → `_`）视觉正常，而码灵拿到的是**原始 markdown 源码**，
//! 且码灵把需求正文当**纯文本**消费（composer 预填 + agent 首轮 prompt），不经过 markdown
//! 渲染——转义层从未被解掉。本模块在 [`RemoteTask::requirement_text`] 边界补上这一层解码：
//! 按 CommonMark 转义语义反转 tiptap 的序列化转义，其余 markdown 语法（`**`、`#`、链接等）
//! 原样保留，只消除转义、不做 markdown → 文本的结构转换。
//!
//! 规则（CommonMark 对齐）：
//! - 围栏代码块（``` / ~~~，≤3 空格缩进、≥3 字符开栏；闭栏同字符且长度 ≥ 开栏）内容逐字保留；
//! - 行内代码 span（反引号 run，等长 run 闭合）内容逐字保留；未找到等长闭合 run 时开栏
//!   按字面处理、其后正文照常解码（避免单个杂散反引号吞掉全文解码）；
//! - 其余位置 `\` + ASCII 标点 → 标点字符；`\` + 其他（含行尾硬换行、非 ASCII、Windows
//!   路径 `\U` `\d` 等）→ 反斜杠原样保留；
//! - 代码 span 不跨段落（空行）配对、不跨围栏——段落/围栏边界处未闭合的开栏回退为字面。
//!
//! 文档化简化（对 tiptap 产出无影响）：不处理 4 空格缩进代码块与 setext 标题（tiptap 代码块
//! 序列化为围栏式）；不校验 ``` 围栏 info 串是否含反引号。

/// markdown 源码 → 纯文本：消除 CommonMark 反斜杠转义（代码块 / 行内代码内逐字保留）。
///
/// 见模块文档。纯函数、无配置；调用方为 [`super::client::RemoteTask::requirement_text`]。
pub(crate) fn markdown_source_to_plain_text(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    // 开栏围栏（围栏字符 + 开栏长度）；围栏内逐字直通直到闭栏。
    let mut fence: Option<(char, usize)> = None;
    // 当前段落缓冲（原文行）。段落 = 围栏与空行之间的连续非空行；代码 span 只在段内配对。
    let mut para: Vec<&str> = Vec::new();

    for line in source.split_inclusive('\n') {
        if let Some((fence_char, open_len)) = fence {
            out.push_str(line);
            if is_closing_fence(line, fence_char, open_len) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = opening_fence(line) {
            // 围栏打断段落：段内未闭合的反引号回退为字面（其内容已在段落 flush 中按字面处理）。
            flush_paragraph(&mut para, &mut out);
            fence = Some(marker);
            out.push_str(line);
            continue;
        }
        if line.trim().is_empty() {
            // 空行结束段落：代码 span 不跨空行配对。
            flush_paragraph(&mut para, &mut out);
            out.push_str(line);
            continue;
        }
        para.push(line);
    }
    flush_paragraph(&mut para, &mut out);
    out
}

/// flush 段落缓冲：整段做行内解码后写回。
fn flush_paragraph(para: &mut Vec<&str>, out: &mut String) {
    if para.is_empty() {
        return;
    }
    let joined = para.concat();
    decode_paragraph(&joined, out);
    para.clear();
}

/// 行首开栏围栏：≤3 空格缩进 + ≥3 连续 `` ` `` 或 `~`。返回（围栏字符, 长度）。
fn opening_fence(line: &str) -> Option<(char, usize)> {
    let body = line.trim_end_matches(['\n', '\r']);
    let indent = body.len() - body.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &body[indent..];
    let first = rest.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let run = rest.chars().take_while(|&c| c == first).count();
    (run >= 3).then_some((first, run))
}

/// 闭栏：≤3 空格缩进 + 同字符 run ≥ 开栏长度 + 其后仅空白。
fn is_closing_fence(line: &str, fence_char: char, open_len: usize) -> bool {
    let body = line.trim_end_matches(['\n', '\r']);
    let trimmed = body.trim_start_matches(' ');
    let indent = body.len() - trimmed.len();
    if indent > 3 {
        return false;
    }
    let run = trimmed.chars().take_while(|&c| c == fence_char).count();
    // fence_char 是 ASCII，char 数即字节数，切片安全。
    run >= open_len && trimmed[run..].trim().is_empty()
}

/// 段内行内解码：`\` + ASCII 标点 → 标点；行内代码 span（等长反引号 run 闭合）逐字保留；
/// 未闭合的开栏 run 按字面处理、其后照常解码。
fn decode_paragraph(para: &str, out: &mut String) {
    let chars: Vec<char> = para.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            if chars.get(i + 1).is_some_and(char::is_ascii_punctuation) {
                out.push(chars[i + 1]);
                i += 2;
            } else {
                out.push('\\');
                i += 1;
            }
        } else if c == '`' {
            let start = i;
            let mut run = 0usize;
            while i < chars.len() && chars[i] == '`' {
                run += 1;
                i += 1;
            }
            match find_closing_run(&chars, i, run) {
                // opener + 内容 + closer 整体逐字保留（span 内 `\` 是用户字面输入）。
                Some(close_start) => {
                    for &ch in &chars[start..close_start + run] {
                        out.push(ch);
                    }
                    i = close_start + run;
                }
                // 未闭合：开栏按字面输出，其后从当前位置照常解码。
                None => {
                    for _ in 0..run {
                        out.push('`');
                    }
                }
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
}

/// 从 `from` 起向后找长度恰为 `run` 的反引号 run（行内代码闭合规则：等长才闭合）。
/// 返回闭合 run 起始下标；其他长度的 run 是内容、跳过继续找。
fn find_closing_run(chars: &[char], from: usize, run: usize) -> Option<usize> {
    let mut j = from;
    while j < chars.len() {
        if chars[j] == '`' {
            let start = j;
            let mut r = 0usize;
            while j < chars.len() && chars[j] == '`' {
                r += 1;
                j += 1;
            }
            if r == run {
                return Some(start);
            }
        } else {
            j += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_tiptap_escaped_punctuation() {
        // tiptap @tiptap/markdown escapeMarkdownSyntax 会转义 \`*[]~ 等 ASCII 标点。
        assert_eq!(
            markdown_source_to_plain_text(r"包名 my\_app\_1 部署 dev\_cluster"),
            "包名 my_app_1 部署 dev_cluster",
        );
        assert_eq!(
            markdown_source_to_plain_text(r"\*\*重点\*\* \[链接\] \~倾斜\~ \#\# 不是标题"),
            "**重点** [链接] ~倾斜~ ## 不是标题",
        );
        // 转义反斜杠本身：`\\` → `\`，且 `\`` 解出的反引号不触发代码 span。
        assert_eq!(
            markdown_source_to_plain_text(r"\`code\_x\` 与 \\ 路径"),
            "`code_x` 与 \\ 路径",
        );
    }

    #[test]
    fn keeps_backslash_before_non_punctuation() {
        // Windows 路径与非 ASCII 前的反斜杠不是 CommonMark 转义，原样保留。
        assert_eq!(
            markdown_source_to_plain_text(r"C:\Users\dev\工作目录\_log"),
            "C:\\Users\\dev\\工作目录_log",
        );
    }

    #[test]
    fn decodes_consecutive_escapes_left_to_right() {
        // `\\` 先解为单个 `\`，随后的 `\_` 再解为 `_` → 输出 `\_`。
        assert_eq!(markdown_source_to_plain_text(r"a\\\_b"), r"a\_b");
    }

    #[test]
    fn keeps_code_span_verbatim() {
        assert_eq!(
            markdown_source_to_plain_text(r"先跑 `build\_all.sh` 再部署 dev\_cluster"),
            r"先跑 `build\_all.sh` 再部署 dev_cluster",
        );
        // 等长 run 才闭合；内容里其他长度的 run 是内容、不闭合。
        assert_eq!(
            markdown_source_to_plain_text(r"``a \`b`` 尾\_y"),
            r"``a \`b`` 尾_y",
        );
    }

    #[test]
    fn unclosed_backtick_falls_back_to_normal_decoding() {
        // 段内找不到等长闭合 run → 开栏按字面，其后正文照常解码（杂散反引号不吞掉全文解码）。
        assert_eq!(
            markdown_source_to_plain_text(r"错误：`EACCES 然后 my\_app"),
            "错误：`EACCES 然后 my_app",
        );
    }

    #[test]
    fn code_span_does_not_cross_paragraph_or_fence() {
        // 空行分段：段 1 未闭合的开栏不与段 2 配对，两段各自按字面 + 照常解码。
        assert_eq!(
            markdown_source_to_plain_text("`a\n\nb`\\_x"),
            "`a\n\nb`_x",
        );
        // 围栏打断段落：围栏前的未闭合开栏按字面，围栏内容逐字，围栏后照常解码。
        assert_eq!(
            markdown_source_to_plain_text("`open\n```\n\\_in fence\n```\n\\_after"),
            "`open\n```\n\\_in fence\n```\n_after",
        );
    }

    #[test]
    fn keeps_fenced_code_block_verbatim() {
        let src = "正文\\_x\n```rust\nfn a\\_b() {}\n```\n尾\\_y\n";
        assert_eq!(
            markdown_source_to_plain_text(src),
            "正文_x\n```rust\nfn a\\_b() {}\n```\n尾_y\n",
        );
    }

    #[test]
    fn fence_close_requires_same_char_and_min_length() {
        // ``` 不闭合 ~~~ 围栏（闭栏须同字符）。
        assert_eq!(
            markdown_source_to_plain_text("~~~\n\\_keep\n```\n~~~\n\\_after"),
            "~~~\n\\_keep\n```\n~~~\n_after",
        );
        // 闭栏长度不足开栏（3 < 4）→ 仍是内容；闭栏后允许尾随空白。
        assert_eq!(
            markdown_source_to_plain_text("````\n\\_a\n```\n\\_b\n````  \n\\_end"),
            "````\n\\_a\n```\n\\_b\n````  \n_end",
        );
    }

    #[test]
    fn fence_allows_up_to_three_space_indent() {
        // 1-3 空格缩进的开/闭栏仍识别；4 空格按普通文本处理（文档化简化：不做缩进代码块）。
        assert_eq!(
            markdown_source_to_plain_text("  ```\n\\_in\n  ```\n\\_out"),
            "  ```\n\\_in\n  ```\n_out",
        );
        assert_eq!(
            markdown_source_to_plain_text("    ```\n\\_not fence"),
            "    ```\n_not fence",
        );
    }

    #[test]
    fn keeps_backslash_at_line_end() {
        // 行尾反斜杠是硬换行语义，下一个字符是换行符、非 ASCII 标点 → 保留。
        assert_eq!(
            markdown_source_to_plain_text("行一\\\n行二\\_x"),
            "行一\\\n行二_x",
        );
    }

    #[test]
    fn plain_and_crlf_text_unchanged() {
        assert_eq!(markdown_source_to_plain_text(""), "");
        assert_eq!(
            markdown_source_to_plain_text("普通中文 English 123 ¥ 常规文本"),
            "普通中文 English 123 ¥ 常规文本",
        );
        // CRLF 行尾：`\r` 非标点，`\` 保留；行内转义照常解码。
        assert_eq!(
            markdown_source_to_plain_text("a\\_b\r\nc\\_d\r\n"),
            "a_b\r\nc_d\r\n",
        );
    }
}
