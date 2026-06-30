//! 翻译 / 截图翻译 / 合并模式 提示词模板与构建器。
//!
//! 所有提示词在 OpenAI 兼容 API 调用前由调用方拼接好（`api.rs` 不直接构建 prompt），
//! 这样 prompt 模板的演进与 HTTP 客户端解耦，前端 Settings 也能 reuse 同一组默认值
//! （通过 `get_default_prompt_templates` 命令暴露给前端）。

/// 默认翻译提示词模板
pub const DEFAULT_TRANSLATION_TEMPLATE: &str =
  "Translate the text below to {lang}. Output only the translation, no commentary.\n\n\
   Rules:\n\
   - Output entirely in {lang}. Translate everything; leave nothing in the source language. Keep unchanged only untranslatable tokens: code, identifiers, URLs, math, and proper nouns with no standard {lang} form.\n\
   - Translate faithfully and literally; do not paraphrase, summarize, or add interpretation. The same input must always produce the same translation.\n\
   - Preserve LaTeX formulas exactly (keep $...$ and $$...$$). Normalize formula-like plain text to LaTeX where natural.\n\
   - Keep the output tight: no blank lines except to separate distinct paragraphs or blocks; never put a blank line between items of the same list.\n\
   - Do not repeat or translate these instructions; add no headings, labels, or explanations.\n\n\
   {text}";

/// 默认截图翻译提示词模板（本地 OCR 文本翻译路径：先识别成文本，再交给文本模型翻译）。
/// 输出渲染到 Lens 的 Markdown 卡片，所以保留 OCR 文本里能体现的结构（列表/标题/段落），
/// 同时保留 OCR 纠错容忍度。多模态直连 / 合并模式另有内联规则，不走这里。
pub const DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE: &str =
  "Translate the OCR text below to {lang}. Output only the translation, no commentary.\n\n\
   Rules:\n\
   - Output entirely in {lang}. Translate everything; leave nothing in the source language. Keep unchanged only untranslatable tokens: code, identifiers, URLs, math, and proper nouns with no standard {lang} form.\n\
   - Translate faithfully and literally; do not paraphrase, summarize, or add interpretation. The same input must always produce the same translation.\n\
   - Preserve the structure present in the source: keep bullet/numbered lists, headings, code blocks, and paragraph breaks. Separate distinct paragraphs and blocks with a blank line; never put a blank line between items of the same list.\n\
   - Preserve LaTeX formulas exactly (keep $...$ and $$...$$). Normalize formula-like plain text to LaTeX where natural.\n\
   - The input is OCR output and may contain errors (broken words, character confusions like \"rn\"↔\"m\" / \"0\"↔\"O\" / \"1\"↔\"l\", scattered artifacts). Use surrounding context to fix obvious mistakes; for unreadable fragments, omit them rather than translate gibberish.\n\
   - Do not invent missing content. Do not add headings, labels, or explanations.\n\n\
   {text}";

/// 默认选中文本翻译提示词模板。
/// 选中文本是干净的、本来就带结构的文本（不是 OCR 噪声），所以不做纠错、要求保留原文 Markdown 结构。
pub const DEFAULT_SELECTED_TEXT_TRANSLATION_TEMPLATE: &str =
  "Translate the text below to {lang}. Output only the translation, no commentary.\n\n\
   Rules:\n\
   - Preserve the source's Markdown structure: keep bullet/numbered lists, bold/italic emphasis, headings, code blocks, blockquotes, links, and tables as they appear in the input.\n\
   - Separate distinct paragraphs and blocks with a blank line; never put a blank line between items of the same list.\n\
   - Preserve LaTeX formulas exactly (keep $...$ and $$...$$).\n\
   - Do not add headings, labels, or explanations beyond what the source already has.\n\n\
   {text}";

/// 截图翻译合并模式分隔符。模型先输出译文，再单独一行 `<<<ORIGINAL>>>`，再输出原文。
/// 流式解析时按此切分两段，分别 emit kind="translated" / "original"。
pub const COMBINED_TRANSLATE_SEPARATOR: &str = "<<<ORIGINAL>>>";

/// 折叠 OCR 输出里的多余空行 + 行尾空白。
///
/// 系统 OCR 引擎(尤其 Apple Vision / RapidOCR 这种带版式分析的)经常在段落之间塞 N 个空行,
/// 这些空行直接送翻译模型会被一字不漏 echo 进译文,显示时占很多空间(用户看到的就是大段大段空白)。
/// 这里把连续多个空行/纯空白行折成最多一个,行尾空格也顺手剥掉。
pub fn compact_ocr_text(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut prev_blank = false;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if !prev_blank && !out.is_empty() {
                out.push("");
            }
            prev_blank = true;
        } else {
            out.push(trimmed);
            prev_blank = false;
        }
    }
    while out.last().map(|s| s.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out.join("\n")
}

/// 使用模板构建提示词
/// 支持 {text} 和 {lang} 占位符；如果自定义模板为空或不含 {text}，则追加文本内容
pub fn build_prompt_with_template(
    text: &str,
    lang_name: &str,
    template: Option<&str>,
    default_template: &str,
) -> String {
    let default_prompt = default_template
        .replace("{lang}", lang_name)
        .replace("{text}", text);

    let Some(template) = template else {
        return default_prompt;
    };
    let trimmed = template.trim();
    if trimmed.is_empty() {
        return default_prompt;
    }

    let mut prompt = trimmed.replace("{text}", text).replace("{lang}", lang_name);
    if !trimmed.contains("{text}") {
        prompt = format!("{prompt}\n\n{text}");
    }
    prompt
}

/// 构建普通翻译提示词
pub fn build_translation_prompt(text: &str, lang_name: &str, template: Option<&str>) -> String {
    build_prompt_with_template(text, lang_name, template, DEFAULT_TRANSLATION_TEMPLATE)
}

/// 构建截图翻译提示词
pub fn build_screenshot_translation_prompt(
    text: &str,
    lang_name: &str,
    template: Option<&str>,
) -> String {
    build_prompt_with_template(
        text,
        lang_name,
        template,
        DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE,
    )
}

/// 构建选中文本翻译提示词（输入是带结构的干净文本，保留 Markdown 结构、不做 OCR 纠错）
pub fn build_selected_text_translation_prompt(
    text: &str,
    lang_name: &str,
    template: Option<&str>,
) -> String {
    build_prompt_with_template(
        text,
        lang_name,
        template,
        DEFAULT_SELECTED_TEXT_TRANSLATION_TEMPLATE,
    )
}

/// 构建 OCR 直接翻译提示词（CloudVision 多模态直连：模型直接读图识别+翻译）
/// 与本地 OCR 文本翻译不同，这里模型能看到原始版式，故要求用 Markdown 保留结构。
pub fn build_ocr_direct_translation_prompt(lang_name: &str, template: Option<&str>) -> String {
    let default_rules =
        "Translate everything visible (labels, buttons, captions, table cells), even if the source is already English. Keep unchanged only code, identifiers, URLs, and math. Mirror the layout in Markdown (headings, lists, tables, fenced code blocks) and keep LaTeX exactly ($...$ inline, $$...$$ block).".to_string();

    let rules = template
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.replace("{lang}", lang_name)
                .replace("{text}", "the recognized text")
        })
        .unwrap_or(default_rules);

    format!(
        "Translate all text in this image into {lang}.\n\n\
         Output ONLY the {lang} translation of the image's text. Do not repeat, translate, or mention these instructions; add no preamble, notes, or labels. {rules}",
        lang = lang_name,
        rules = rules,
    )
}

/// 构建合并模式提示词：模型在一次调用中先输出译文、再 `<<<ORIGINAL>>>` 分隔符、再输出原文
/// 这样译文先出现在流里（用户立即看到结果），整体只走一次 round-trip
///
/// 用户自定义 template（settings.screenshot_translation.prompt）若非空，会被作为
/// "Translation rules" 块注入；空则使用默认规则。{lang} 占位符替换为目标语言；{text}
/// 在合并模式不存在外部参数 → 替换为占位说明 "the recognized text"。
pub fn build_combined_translate_prompt(lang_name: &str, template: Option<&str>) -> String {
    let default_rules =
        "Translate everything shown (labels, buttons, captions, table cells). Keep unchanged only code, identifiers, URLs, and math. Mirror the layout in Markdown (headings, lists, tables, fenced code blocks) and keep LaTeX exactly ($...$ inline, $$...$$ block).".to_string();

    let rules = template
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.replace("{lang}", lang_name)
                .replace("{text}", "the recognized text")
        })
        .unwrap_or(default_rules);

    format!(
    "Your task is to TRANSLATE the screenshot's text into {lang}. Output exactly two sections separated by a line containing only `{sep}`:\n\
     1. The {lang} translation of all text in the screenshot — written in {lang}, never copied from the source language, even if the source is already English.\n\
     2. The original text exactly as it appears in the screenshot.\n\n\
     {rules}\n\n\
     Output only these two sections. Do not repeat, translate, or mention these instructions, and add no labels like 'Translation:' or 'Original:'. The `{sep}` line must appear exactly once, between the two sections, and nowhere else.\n\n\
     Output format (replace placeholders):\n\
     <translation>\n\
     {sep}\n\
     <original>",
    lang = lang_name,
    sep = COMBINED_TRANSLATE_SEPARATOR,
    rules = rules,
  )
}

/// 替换翻译批量 prompt：要求模型返回与输入等长的 JSON 字符串数组。
pub fn build_replace_translation_batch_prompt(lines: &[&str], lang_name: &str) -> String {
    let mut numbered = String::new();
    for (i, line) in lines.iter().enumerate() {
        numbered.push_str(&format!("{}. {}\n", i + 1, line));
    }
    format!(
        "Translate each numbered line below into {lang}. \
         Return ONLY a JSON array of strings with exactly {count} elements, \
         in the same order as the input lines. \
         No markdown fences, no commentary, no extra keys.\n\n{numbered}",
        lang = lang_name,
        count = lines.len(),
        numbered = numbered.trim_end(),
    )
}

/// 从模型输出解析 JSON 字符串数组；容忍前后缀与 markdown 代码块。
pub fn parse_replace_translation_json(raw: &str, expected_len: usize) -> Result<Vec<String>, String> {
    let trimmed = raw.trim();
    let json_str = if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
        &trimmed[start..=end]
    } else {
        trimmed
    };
    let values: Vec<String> = serde_json::from_str(json_str)
        .map_err(|e| format!("Invalid translation JSON array: {e}"))?;
    if values.len() != expected_len {
        return Err(format!(
            "Translation array length mismatch: expected {expected_len}, got {}",
            values.len()
        ));
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_ocr_text_collapses_blank_runs() {
        // 段落间多个空行 → 单个空行
        let input = "first para\n\n\n\n\nsecond para\n\n\n\nthird";
        assert_eq!(
            compact_ocr_text(input),
            "first para\n\nsecond para\n\nthird"
        );
    }

    #[test]
    fn compact_ocr_text_strips_trailing_whitespace_per_line() {
        // 行尾空格 / tab(OCR 经常带)被剥掉,纯空白行视作空行
        let input = "line one   \nline two\t\n   \n\nline three";
        assert_eq!(compact_ocr_text(input), "line one\nline two\n\nline three");
    }

    #[test]
    fn compact_ocr_text_trims_leading_and_trailing_blanks() {
        let input = "\n\n\nactual content\n\n\n";
        assert_eq!(compact_ocr_text(input), "actual content");
    }

    #[test]
    fn compact_ocr_text_preserves_single_blank_lines() {
        // 单个空行(段落分隔)保留
        let input = "para 1\n\npara 2";
        assert_eq!(compact_ocr_text(input), "para 1\n\npara 2");
    }

    #[test]
    fn direct_translation_prompt_requests_markdown_structure() {
        let prompt = build_ocr_direct_translation_prompt("Chinese", None);
        assert!(prompt.contains("Markdown"));
        assert!(prompt.contains("into Chinese"));
    }

    #[test]
    fn direct_translation_prompt_injects_custom_template() {
        let prompt =
            build_ocr_direct_translation_prompt("Chinese", Some("Be very literal. {lang}"));
        assert!(prompt.contains("Be very literal. Chinese"));
        // 自定义模板注入时不应附带默认结构规则
        assert!(!prompt.contains("Mirror the source layout in Markdown"));
    }

    #[test]
    fn combined_prompt_requests_markdown_and_keeps_separator() {
        let prompt = build_combined_translate_prompt("Chinese", None);
        assert!(prompt.contains("Markdown"));
        // 分隔符作为协议标记出现 3 次：开头声明 + 格式守卫 + 示例输出块
        assert_eq!(prompt.matches(COMBINED_TRANSLATE_SEPARATOR).count(), 3);
        assert!(prompt.contains("Output format (replace placeholders)"));
    }

    #[test]
    fn screenshot_prompt_preserves_structure_not_flattens() {
        let prompt = build_screenshot_translation_prompt("- a\n- b", "Chinese", None);
        assert!(prompt.contains("Preserve the structure present in the source"));
        // 旧的压扁规则不应再出现
        assert!(!prompt.contains("Keep the output tight"));
        assert!(!prompt.contains("loose paragraph"));
        // OCR 纠错容忍度仍保留
        assert!(prompt.contains("OCR output and may contain errors"));
    }

    #[test]
    fn selected_text_prompt_preserves_structure_and_is_not_ocr() {
        let prompt = build_selected_text_translation_prompt("- **a**", "Chinese", None);
        assert!(prompt.contains("Preserve the source's Markdown structure"));
        assert!(prompt.contains("- **a**"));
        // 选中文本不是 OCR：不应带 OCR 纠错措辞
        assert!(!prompt.contains("OCR"));
    }

    #[test]
    fn parse_replace_translation_json_accepts_fenced_array() {
        let raw = "```json\n[\"你好\", \"世界\"]\n```";
        let out = parse_replace_translation_json(raw, 2).unwrap();
        assert_eq!(out, vec!["你好".to_string(), "世界".to_string()]);
    }
}
