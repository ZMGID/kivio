//! 翻译 / 截图翻译 / 合并模式 提示词模板与构建器。
//!
//! 所有提示词在 OpenAI 兼容 API 调用前由调用方拼接好（`api.rs` 不直接构建 prompt），
//! 这样 prompt 模板的演进与 HTTP 客户端解耦，前端 Settings 也能 reuse 同一组默认值
//! （通过 `get_default_prompt_templates` 命令暴露给前端）。

/// 默认翻译提示词模板
pub const DEFAULT_TRANSLATION_TEMPLATE: &str =
  "Translate the text below to {lang}. Output only the translation, no commentary.\n\n\
   Rules:\n\
   - Preserve LaTeX formulas exactly (keep $...$ and $$...$$). Normalize formula-like plain text to LaTeX where natural.\n\
   - Keep the output tight: do not output blank lines. Use a single newline only for necessary list items, table rows, code/math blocks, or clear paragraph boundaries.\n\
   - Do not use Markdown's loose paragraph style. Never put an empty line between numbered or bulleted list items.\n\
   - If the input looks like OCR output (broken words, garbled chars, scattered artifacts), use surrounding context to fix obvious errors before translating; for clearly unreadable fragments, omit them rather than guess.\n\
   - Do not add headings, labels, or explanations.\n\n\
   {text}";

/// 默认截图翻译提示词模板
pub const DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE: &str =
  "Translate the OCR text below to {lang}. Output only the translation, no commentary.\n\n\
   Rules:\n\
   - Preserve LaTeX formulas exactly (keep $...$ and $$...$$). Normalize formula-like plain text to LaTeX where natural.\n\
   - Keep the output tight: do not output blank lines. Use a single newline only for necessary list items, table rows, code/math blocks, or clear paragraph boundaries.\n\
   - Do not use Markdown's loose paragraph style. Never put an empty line between numbered or bulleted list items.\n\
   - The input is OCR output and may contain errors (broken words, character confusions like \"rn\"↔\"m\" / \"0\"↔\"O\" / \"1\"↔\"l\", scattered artifacts). Use surrounding context to fix obvious mistakes; for unreadable fragments, omit them rather than translate gibberish.\n\
   - Do not invent missing content. Do not add headings, labels, or explanations.\n\n\
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

/// 构建 OCR 直接翻译提示词
/// 将截图翻译模板嵌入到 OCR 指令中，让模型一次性完成识别和翻译
pub fn build_ocr_direct_translation_prompt(lang_name: &str, template: Option<&str>) -> String {
    let instruction = build_screenshot_translation_prompt(
        "the text content recognized from this image",
        lang_name,
        template,
    );
    format!(
    "Read all text in this image, then follow this instruction exactly:\n{}\n\nOutput only the final translated text. Do not output blank lines; keep lists and paragraphs compact. For mathematical formulas, keep LaTeX notation ($...$ or $$...$$).",
    instruction
  )
}

/// 构建合并模式提示词：模型在一次调用中先输出译文、再 `<<<ORIGINAL>>>` 分隔符、再输出原文
/// 这样译文先出现在流里（用户立即看到结果），整体只走一次 round-trip
///
/// 用户自定义 template（settings.screenshot_translation.prompt）若非空，会被作为
/// "Translation rules" 块注入；空则使用默认规则。{lang} 占位符替换为目标语言；{text}
/// 在合并模式不存在外部参数 → 替换为占位说明 "the recognized text"。
pub fn build_combined_translate_prompt(lang_name: &str, template: Option<&str>) -> String {
    const DEFAULT_RULES: &str = "- Preserve LaTeX formulas exactly ($...$ inline, $$...$$ block).\n\
     - Keep the translation tight: do not output blank lines. Use a single newline only for necessary list items, table rows, code/math blocks, or clear paragraph boundaries.\n\
     - Do not use Markdown's loose paragraph style. Never put an empty line between numbered or bulleted list items.\n\
     - Correct obvious OCR mistakes using context; for unreadable fragments omit rather than guess.\n\
     - No commentary, no section headers, no labels.";

    let rules = template
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.replace("{lang}", lang_name)
                .replace("{text}", "the recognized text")
        })
        .unwrap_or_else(|| DEFAULT_RULES.to_string());

    format!(
    "Read this screenshot. Output two sections in this exact order, separated by a line containing only `{sep}`:\n\n\
     1. Translation in {lang}: a faithful translation of all text shown in the screenshot.\n\
     2. Original recognized text exactly as it appears in the screenshot.\n\n\
     Translation rules:\n{rules}\n\n\
     Format guard:\n\
     - The line `{sep}` must appear exactly once on its own line, between the two sections.\n\
     - Do not add blank lines before or after `{sep}`. Do not add blank lines inside the translation.\n\
     - Keep the original recognized text compact too: preserve line breaks when useful, but collapse empty lines.\n\
     - No commentary, no labels like 'Translation:' or 'Original:'.\n\n\
     Output format (replace placeholders):\n\
     <translation>\n\
     {sep}\n\
     <original>",
    lang = lang_name,
    sep = COMBINED_TRANSLATE_SEPARATOR,
    rules = rules,
  )
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
}
