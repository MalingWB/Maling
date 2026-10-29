//! multica 需求正文的 markdown 源码 → 纯文本解码（开发设计 M5-bx 方案 A + M5-bz HTML 层）。
//!
//! 根因：multica web issue 编辑器（ContentEditor，tiptap）保存走
//! `@tiptap/markdown` 序列化，正文中 `_` `*` `[` `]` `~` 等会被转义成 `\_` `\*` 存库，
//! 文本节点还会做 HTML 实体编码（`&` `&nbsp;` 等），空段落序列化为 `&nbsp;`；DPMS 同步的
//! HTML 正文则以 `<p>` 块等 HTML 岛屿形式混入。web 端展示时再解析 markdown / 渲染 HTML
//! 视觉正常，而码灵拿到的是**原始源码**，且码灵把需求正文当**纯文本**消费（composer 预填 +
//! agent 首轮 prompt），不经过渲染——转义层与 HTML 层从未被解掉。本模块在
//! [`RemoteTask::requirement_text`] 边界补上这两层解码：按 CommonMark 语义反转 tiptap 的
//! 序列化产物，其余 markdown 语法（`**`、`#`、链接等）原样保留，只消除转义与 HTML 标记、
//! 不做 markdown → 文本的结构转换。
//!
//! 规则（CommonMark / HTML 渲染器口径对齐，逐字符单遍扫描，按源位置定先后）：
//! - 围栏代码块（``` / ~~~，≤3 空格缩进、≥3 字符开栏；闭栏同字符且长度 ≥ 开栏）内容逐字保留；
//! - 行内代码 span（反引号 run，等长 run 闭合）内容逐字保留；未找到等长闭合 run 时开栏
//!   按字面处理、其后正文照常解码（避免单个杂散反引号吞掉全文解码）；
//! - 其余位置 `\` + ASCII 标点 → 标点字符；`\` + 其他（含行尾硬换行、非 ASCII、Windows
//!   路径 `\U` `\d` 等）→ 反斜杠原样保留；
//! - 原始 HTML 标签（CommonMark 标签语法，属性区含引号串）按渲染器口径剥离：`<br>` →
//!   换行；块级标签 → 段落分隔（行首/紧随换行后不叠加）；行内标签剥壳保留内容；注释消失。
//!   `&lt;p&gt;` 与 `\<p>` 解出的是**用户字面文本** `<p>`，不再当标签二次剥离（单遍扫描
//!   天然保证）；非标签语法（如 `a<b 且 c>d`）的 `<` 按字面保留；
//! - HTML 实体解码：tiptap/HTML 常见封闭命名集（`amp` `lt` `gt` `quot` `nbsp`）+ 十进制/
//!   十六进制数字实体；未匹配按字面保留；单遍不回环（`&amp;lt;` → 字面 `&lt;`）；
//! - 代码 span 不跨段落（空行）配对、不跨围栏——段落/围栏边界处未闭合的开栏回退为字面。
//!
//! 文档化简化（对 tiptap 产出无影响）：不处理 4 空格缩进代码块与 setext 标题（tiptap 代码块
//! 序列化为围栏式）；不校验 ``` 围栏 info 串是否含反引号；HTML 注释未闭合时保守按字面；
//! 只处理开/闭标签与注释，不处理 PI/CDATA/声明；命名实体只收封闭集（未知实体按字面）。

/// markdown 源码 → 纯文本：消除 CommonMark 反斜杠转义与 HTML 标记/实体
/// （代码块 / 行内代码内逐字保留）。
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

/// 块级标签集（HTML 规范块级中影响文本布局的常见子集）：纯文本里表达为段落分隔（换行）。
/// 不在此集合与 `br` 中的标签按行内处理：剥壳保留内容。
const BLOCK_LEVEL_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "caption",
    "dd",
    "details",
    "dialog",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
];

/// HTML 标签的纯文本投影方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HtmlTagKind {
    /// `<br>`：显式换行，始终生效（连续 `<br>` 各自换行，保留空行意图）。
    Break,
    /// 块级标签：段落分隔（换行）；行首/紧随换行后不叠加，防裸空行。
    Block,
    /// 行内标签：剥壳丢弃标签本身，内容照常解码。
    Inline,
    /// 注释等不可见节点：整体消失。
    Invisible,
}

/// 段内行内解码（逐字符单遍扫描，源位置决定先后，无回环重扫）：`\` + ASCII 标点 → 标点；
/// 行内代码 span（等长反引号 run 闭合）逐字保留；原始 HTML 标签剥离；HTML 实体解码。
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
        } else if c == '<' {
            match match_html_tag(&chars, i) {
                Some((end, kind)) => {
                    match kind {
                        HtmlTagKind::Break => out.push('\n'),
                        HtmlTagKind::Block => {
                            if !out.is_empty() && !out.ends_with('\n') {
                                out.push('\n');
                            }
                        }
                        HtmlTagKind::Inline | HtmlTagKind::Invisible => {}
                    }
                    i = end;
                }
                None => {
                    out.push('<');
                    i += 1;
                }
            }
        } else if c == '&' {
            match decode_entity(&chars, i) {
                Some((ch, end)) => {
                    out.push(ch);
                    i = end;
                }
                None => {
                    out.push('&');
                    i += 1;
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

/// 从 `start`（`chars[start] == '<'`）匹配一个 CommonMark 原始 HTML 标签
/// （开标签 / 闭标签 / 自闭合 / 注释）。返回（标签结束下标, 纯文本投影方式）；
/// 不匹配返回 None（`<` 按字面文本）。
fn match_html_tag(chars: &[char], start: usize) -> Option<(usize, HtmlTagKind)> {
    let mut i = start + 1;
    // HTML 注释：闭合 `-->` 才按注释消失；未闭合保守按字面（不吞后续全文）。
    if chars.get(i) == Some(&'!')
        && chars.get(i + 1) == Some(&'-')
        && chars.get(i + 2) == Some(&'-')
    {
        let mut j = i + 3;
        while j + 3 <= chars.len() {
            if chars[j] == '-' && chars[j + 1] == '-' && chars[j + 2] == '>' {
                return Some((j + 3, HtmlTagKind::Invisible));
            }
            j += 1;
        }
        return None;
    }
    let closing = chars.get(i) == Some(&'/');
    if closing {
        i += 1;
    }
    // 标签名：ASCII 字母开头，后续字母/数字/连字符（CommonMark）。
    if !chars.get(i).is_some_and(char::is_ascii_alphabetic) {
        return None;
    }
    let name_start = i;
    while chars
        .get(i)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '-')
    {
        i += 1;
    }
    let name: String = chars[name_start..i]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    if closing {
        // 闭标签不带属性，仅允许尾随 ASCII 空白后接 `>`。
        while chars.get(i).is_some_and(char::is_ascii_whitespace) {
            i += 1;
        }
        return (chars.get(i) == Some(&'>')).then(|| (i + 1, classify_tag(&name)));
    }
    let end = scan_open_tag_end(chars, i)?;
    Some((end, classify_tag(&name)))
}

/// 按标签名分类纯文本投影：`br` 显式换行；块级集合 → 段落分隔；其余行内。
fn classify_tag(name: &str) -> HtmlTagKind {
    if name == "br" {
        HtmlTagKind::Break
    } else if BLOCK_LEVEL_TAGS.contains(&name) {
        HtmlTagKind::Block
    } else {
        HtmlTagKind::Inline
    }
}

/// 开标签属性区扫描（CommonMark 属性语法）：`(属性 空白*)* 空白? /? >`；
/// attribute = name (`=` value)?，value = 无引号串（不含 ASCII 空白与 `'` `"` `=` `<` `>` `` ` ``）
/// 或单/双引号串（引号内 `>` 不终止标签）。任一步非法返回 None——`<` 按字面文本
/// （如 `a<b 且 c>d` 的 `<b 且 c>` 属性名非法，不是标签）。
fn scan_open_tag_end(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    loop {
        while chars.get(i).is_some_and(char::is_ascii_whitespace) {
            i += 1;
        }
        match chars.get(i) {
            Some('>') => return Some(i + 1),
            // 自闭合 `/>`。
            Some('/') => return (chars.get(i + 1) == Some(&'>')).then_some(i + 2),
            Some(c) if c.is_ascii_alphabetic() || *c == '_' || *c == ':' => {
                i += 1;
                while chars.get(i).is_some_and(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '.' | '-')
                }) {
                    i += 1;
                }
                if chars.get(i) == Some(&'=') {
                    i += 1;
                    match chars.get(i) {
                        // 引号串：引号内的 `>` 是属性值、不终止标签。
                        Some(q @ ('"' | '\'')) => {
                            i += 1;
                            while chars.get(i).is_some_and(|&c| c != *q) {
                                i += 1;
                            }
                            chars.get(i)?; // 引号未闭合
                            i += 1;
                        }
                        Some(c)
                            if !c.is_ascii_whitespace()
                                && !matches!(c, '"' | '\'' | '=' | '<' | '>' | '`') =>
                        {
                            i += 1;
                            while chars.get(i).is_some_and(|c| {
                                !c.is_ascii_whitespace()
                                    && !matches!(c, '"' | '\'' | '=' | '<' | '>' | '`')
                            }) {
                                i += 1;
                            }
                        }
                        // `=` 后缺值或值以非法字符开头。
                        _ => return None,
                    }
                }
                // 无 `=`：布尔属性，继续下一轮。
            }
            // 属性名非法起始（含非 ASCII 的正文文字）→ 不是标签。
            _ => return None,
        }
    }
}

/// 解码 `start`（`chars[start] == '&'`）处的 HTML 实体：tiptap/HTML 常见封闭命名集
/// （`amp` `lt` `gt` `quot` `nbsp`）+ 十进制 `&#N;` / 十六进制 `&#xN;` 数字实体。
/// 返回（解码字符, 实体结束下标）；未匹配返回 None（`&` 按字面文本，如未知实体、
/// 缺分号、超出 Unicode 标量范围的数字实体）。单遍解码不回环：`&amp;lt;` 解出字面 `&lt;`。
fn decode_entity(chars: &[char], start: usize) -> Option<(char, usize)> {
    let mut i = start + 1;
    let name_start = i;
    while chars
        .get(i)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '#')
    {
        i += 1;
    }
    if chars.get(i) != Some(&';') || i == name_start {
        return None;
    }
    let end = i + 1;
    let name: String = chars[name_start..i].iter().collect();
    let ch = match name.as_str() {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "nbsp" => '\u{00A0}',
        numeric => {
            let digits = numeric.strip_prefix('#')?;
            let value = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse::<u32>().ok()?,
            };
            if value == 0 {
                // NUL 不是合法字符引用（HTML 规范映射 U+FFFD）；保守按字面，不向预填注入 NUL。
                return None;
            }
            return char::from_u32(value).map(|c| (c, end));
        }
    };
    Some((ch, end))
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
        assert_eq!(markdown_source_to_plain_text("`a\n\nb`\\_x"), "`a\n\nb`_x",);
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

    #[test]
    fn strips_html_block_tags_to_newlines() {
        // 整体 HTML 包裹的正文（DPMS 同步 / 序列化产 <p> 块）：块级标签 → 换行，
        // 行首/紧随换行后的块标签换行不再叠加（防正文前后与中间出裸空行）。
        assert_eq!(
            markdown_source_to_plain_text("<p>包名 my\\_app\\_1</p><p>部署 dev\\_cluster</p>"),
            "包名 my_app_1\n部署 dev_cluster\n",
        );
        // 块标签换行与源换行叠加为空行（对应 HTML 块间的视觉分隔），不产生三连换行。
        assert_eq!(
            markdown_source_to_plain_text("<h1>标题</h1>\n<div>内容\\_x</div>"),
            "标题\n\n内容_x\n",
        );
    }

    #[test]
    fn strips_inline_tags_and_break_marks() {
        // 行内标签剥壳保留内容；<br> 变体显式换行（连续 <br> 各自换行，保留空行意图）。
        assert_eq!(
            markdown_source_to_plain_text("<span>高亮</span>正文<strong>重点</strong><u>下划</u>"),
            "高亮正文重点下划",
        );
        assert_eq!(
            markdown_source_to_plain_text("第一行<br>第二行<br/>第三行<br />尾"),
            "第一行\n第二行\n第三行\n尾",
        );
        assert_eq!(
            markdown_source_to_plain_text("段间<br><br>空行"),
            "段间\n\n空行",
        );
    }

    #[test]
    fn decodes_html_entities() {
        // tiptap encodeHtmlEntities 产 &amp;/&lt;/&gt;；空段落标记 &nbsp;；数字/十六进制实体。
        assert_eq!(
            markdown_source_to_plain_text("a &amp; b &lt;c&gt; &quot;d&quot;"),
            "a & b <c> \"d\"",
        );
        assert_eq!(markdown_source_to_plain_text("空&nbsp;格"), "空\u{00A0}格");
        assert_eq!(markdown_source_to_plain_text("&#65;&#x4e2d;"), "A中",);
        // 单遍解码不回环：&amp;lt; → 字面 &lt;（与浏览器口径一致）。
        assert_eq!(markdown_source_to_plain_text("&amp;lt;"), "&lt;");
        // 未知/非法实体按字面保留（&#0;/&#x0; 是非法字符引用，不得解出 NUL 注入预填）。
        assert_eq!(
            markdown_source_to_plain_text("&#xZZ; &#; &#1114112; &#0;&#x0; &nbsp &unknown;"),
            "&#xZZ; &#; &#1114112; &#0;&#x0; &nbsp &unknown;",
        );
    }

    #[test]
    fn entity_and_escape_produced_angle_brackets_are_not_tags() {
        // 渲染器口径：&lt;p&gt; 与 \<p> 都是**用户字面文本**（解码为 <p> 两个字符），
        // 不是 HTML 标记，不参与剥离；只有源内原始 <p> 才是标记。
        assert_eq!(
            markdown_source_to_plain_text("输入 &lt;p&gt; 标签\\_x"),
            "输入 <p> 标签_x",
        );
        assert_eq!(
            markdown_source_to_plain_text(r"字面 \<p> 标签\_x"),
            "字面 <p> 标签_x",
        );
    }

    #[test]
    fn keeps_html_and_entities_verbatim_in_code_regions() {
        let src = "正文<p>\n```\n<p>&nbsp;\n```\n行内 `<b>x</b>` 后\\_y\n";
        assert_eq!(
            markdown_source_to_plain_text(src),
            "正文\n\n```\n<p>&nbsp;\n```\n行内 `<b>x</b>` 后_y\n",
        );
    }

    #[test]
    fn malformed_angle_brackets_stay_literal() {
        // 非 CommonMark 标签语法（名字非法/属性区出现非 ASCII/未闭合）→ < 按字面文本。
        assert_eq!(
            markdown_source_to_plain_text("a<b 且 c>d & 就这样"),
            "a<b 且 c>d & 就这样",
        );
        assert_eq!(
            markdown_source_to_plain_text("比较 3<5 与 6>4"),
            "比较 3<5 与 6>4",
        );
        assert_eq!(markdown_source_to_plain_text("未闭合 <p"), "未闭合 <p");
    }

    #[test]
    fn removes_html_comments_only_when_closed() {
        assert_eq!(markdown_source_to_plain_text("前<!-- 注释 -->后"), "前后",);
        // 未闭合注释保守按字面（不吞后续全文）。
        assert_eq!(
            markdown_source_to_plain_text("未闭合 <!-- 注释"),
            "未闭合 <!-- 注释",
        );
    }

    #[test]
    fn quoted_attribute_value_may_contain_angle_bracket() {
        // 引号内 `>` 不终止标签（CommonMark 属性值规则）。
        assert_eq!(
            markdown_source_to_plain_text(r#"<a title="a>b">链接文字</a> 尾\_y"#),
            "链接文字 尾_y",
        );
    }
}
