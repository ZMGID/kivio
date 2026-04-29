//! 翻译 / 截图翻译 / 合并模式 提示词模板与构建器。
//!
//! 所有提示词在 OpenAI 兼容 API 调用前由调用方拼接好（`api.rs` 不直接构建 prompt），
//! 这样 prompt 模板的演进与 HTTP 客户端解耦，前端 Settings 也能 reuse 同一组默认值
//! （通过 `get_default_prompt_templates` 命令暴露给前端）。

/// 默认翻译提示词模板
pub const DEFAULT_TRANSLATION_TEMPLATE: &str =
  "Translate the following text to {lang}. Output only the translation.\n\nRules:\n- Preserve existing LaTeX formulas exactly (keep $...$ and $$...$$).\n- If formula-like plain text appears, normalize it to proper LaTeX when needed.\n- Keep the original line breaks and list structure when possible.\n- Do not add explanations.\n\n{text}";

/// 默认截图翻译提示词模板
pub const DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE: &str =
  "Translate the OCR text below to {lang}. Output only the translation.\n\nRules:\n- Preserve existing LaTeX formulas exactly (keep $...$ and $$...$$).\n- If formula-like plain text appears, normalize it to proper LaTeX when needed.\n- Keep paragraph and line-break structure from OCR text when possible.\n- Correct only obvious OCR character mistakes; do not invent missing content.\n- Do not add explanations.\n\n{text}";

/// 截图翻译合并模式分隔符。模型先输出译文，再单独一行 `<<<ORIGINAL>>>`，再输出原文。
/// 流式解析时按此切分两段，分别 emit kind="translated" / "original"。
pub const COMBINED_TRANSLATE_SEPARATOR: &str = "<<<ORIGINAL>>>";

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
pub fn build_screenshot_translation_prompt(text: &str, lang_name: &str, template: Option<&str>) -> String {
  build_prompt_with_template(text, lang_name, template, DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE)
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
    "Read all text in this image, then follow this instruction exactly:\n{}\n\nOutput only the final translated text. For mathematical formulas, keep LaTeX notation ($...$ or $$...$$).",
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
  const DEFAULT_RULES: &str = "- Preserve LaTeX formulas ($...$ inline, $$...$$ block).\n\
     - Keep paragraph and line-break structure.\n\
     - Correct only obvious OCR mistakes; do not invent missing content.\n\
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
