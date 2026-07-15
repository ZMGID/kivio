//! kivio-code 的视觉混合（mixer）预分析。
//!
//! 主编码模型常是纯文本（OpenAI 兼容）模型，无法直接「看」用户附上的截图。本模块在一轮 turn
//! 开始前，用一个**显式配置的视觉模型**把每张图片读成客观文字观察，再把这些观察作为附加上下文
//! 注入到本轮消息的克隆里，让纯文本编码模型也能基于截图内容回答。
//!
//! 设计取舍（headless / CLI-local）：
//! - 不复用 `api::call_vision_api`（它绑定 Tauri `AppHandle` + `State<'_, AppState>`，并依赖
//!   `state.explain_images` 的 lens 流式管线）。CLI 持有的是 `Arc<AppState>`，没有 `AppHandle`，
//!   因此这里直接走**与 Tauri 解耦的模型层**（`OpenAiChatProvider` / `AnthropicMessagesProvider`
//!   的 `generate`），与 GUI mixer 的 `call_chat_completion_message` 同一抽象层，仅 `&AppState`。
//! - 仅做「显式视觉模型」路径：未配置则跳过（推一条 Notice），不做自动探测，保持简单。
//! - 注入是**纯函数** [`inject_vision_observations`]，可脱离模型 / 网络单测。

use std::path::{Path, PathBuf};

use base64::{engine::general_purpose, Engine as _};
use serde_json::{json, Value};

use crate::chat::model::{
    generate_request_from_openai_messages, AnthropicMessagesProvider, GenerateOptions,
    GenerateRequestContext, LanguageModelProvider, OpenAiChatProvider, OpenAiResponsesProvider,
};
use crate::settings::{ModelProvider, ProviderApiFormat, Settings};
use crate::state::AppState;

/// 一张图片的视觉分析结果（注入用）。`label` 是用户消息里的占位符（`[Image #N]`）。
pub(crate) struct ImageObservation {
    pub label: String,
    pub analysis: String,
}

/// 根据图片扩展名推断 MIME（与 GUI `image_mime_for_path` 一致；只覆盖 attach 允许的图片类型）。
fn image_mime_for_path(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

/// 视觉副任务的 system 提示（要求客观观察，不替主模型回答）。
fn vision_system_prompt() -> &'static str {
    "You are an auxiliary vision model for a terminal coding agent. Read the user's image(s) and \
     produce concise, objective textual observations for a separate text-only coding model. \
     Describe visible code, error messages, terminal/UI text, diagrams, layout, and any detail \
     relevant to the user's request. Transcribe visible text faithfully. Do not answer the user's \
     final question and do not invent content that is not visible."
}

/// 视觉副任务的 user 提示（携带用户原始文字，帮助聚焦观察）。
fn vision_user_prompt(user_text: &str) -> String {
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        "Describe what is visible in this image for the coding model.".to_string()
    } else {
        format!(
            "The user's message is below. Extract the visual facts the coding model needs to act on it.\n\n{trimmed}"
        )
    }
}

/// 把一张图片读成 base64 的 `image_url` content part（OpenAI 兼容形状；模型层会转成对应 provider 格式）。
fn image_content_part(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read image {}: {e}", path.display()))?;
    let base64 = general_purpose::STANDARD.encode(bytes);
    let mime = image_mime_for_path(path);
    Ok(json!({
        "type": "image_url",
        "image_url": { "url": format!("data:{mime};base64,{base64}") },
    }))
}

/// 用配置的视觉模型分析一张图片，返回客观文字观察。CLI-local：仅需 `&AppState`，无 `AppHandle`。
async fn analyze_one_image(
    state: &AppState,
    provider: &ModelProvider,
    model: &str,
    path: &Path,
    user_text: &str,
) -> Result<String, String> {
    let image_part = image_content_part(path)?;
    let messages = vec![
        json!({ "role": "system", "content": vision_system_prompt() }),
        json!({
            "role": "user",
            "content": [
                image_part,
                { "type": "text", "text": vision_user_prompt(user_text) },
            ],
        }),
    ];

    let request = generate_request_from_openai_messages(
        model,
        messages,
        None,
        GenerateOptions {
            // 关闭思考：视觉副任务只要客观描述，无需推理预算。
            thinking_enabled: false,
            max_tokens: 2048,
            ..GenerateOptions::default()
        },
        "kivio-code-vision-mixer",
        GenerateRequestContext::default(),
    );

    let output = match provider.api_format_kind() {
        ProviderApiFormat::OpenAiChat => {
            OpenAiChatProvider::new(state, provider, 1)
                .generate(request)
                .await
        }
        ProviderApiFormat::AnthropicMessages => {
            AnthropicMessagesProvider::new(state, provider, 1)
                .generate(request)
                .await
        }
        ProviderApiFormat::OpenAiResponses => {
            OpenAiResponsesProvider::new(state, provider, 1)
                .generate(request)
                .await
        }
        ProviderApiFormat::Gemini => {
            crate::chat::model::GeminiProvider::new(state, provider, 1)
                .generate(request)
                .await
        }
    }
    .map_err(|err| err.to_string())?;

    let text = output.text.trim().to_string();
    if text.is_empty() {
        Err("vision model returned empty analysis".to_string())
    } else {
        Ok(text)
    }
}

/// 结果枚举：要么拿到一组观察（连带选用的 provider/model 展示串），要么因无显式视觉模型而跳过。
pub(crate) enum VisionMixerOutcome {
    /// 成功（可能部分图片失败 —— 每条失败也作为 observation 文本记录，便于主模型知情）。
    Analyzed {
        provider_name: String,
        model: String,
        observations: Vec<ImageObservation>,
    },
    /// 未配置显式视觉模型 —— 跳过本轮图片，主模型纯文本继续。
    NoVisionModel,
}

/// 对一批图片跑视觉 mixer 预分析。
///
/// `labels` 与 `image_paths` 一一对应（同序）：`labels[i]` 是 `image_paths[i]` 在用户消息里的
/// `[Image #N]` 占位符。`user_text` 是用户本轮文字（占位符已替换），用于给视觉模型聚焦。
///
/// 行为：
/// - 无显式视觉模型 → `NoVisionModel`（调用方推 Notice + 纯文本继续）。
/// - 否则逐张分析；单张失败不致命 —— 把失败原因作为该图的 observation 文本，继续其余图片。
pub(crate) async fn run_vision_mixer(
    state: &AppState,
    settings: &Settings,
    labels: &[String],
    image_paths: &[PathBuf],
    user_text: &str,
) -> VisionMixerOutcome {
    if !settings.has_explicit_vision_model() {
        return VisionMixerOutcome::NoVisionModel;
    }
    let (provider_id, model) = settings.effective_vision_model();
    let provider = match settings.get_provider(&provider_id) {
        Some(provider) => provider.clone(),
        None => return VisionMixerOutcome::NoVisionModel,
    };
    let provider_name = if provider.name.trim().is_empty() {
        provider.id.clone()
    } else {
        provider.name.clone()
    };

    // 各图分析彼此独立：构造一组 future 并用 `join_all` 并发驱动（与
    // `mcp_setup` / `mcp::registry` 同一模式——借用 `&AppState` / `&provider`，
    // 无需 'static / spawn）。`join_all` **保序**返回，故 observation 顺序仍与
    // `image_paths` 一致，`[Image #N]` 编号不会错位。串行 await 会让多图变成
    // 依次阻塞的多次模型调用，并行后墙钟时间降到最慢的单张。
    let analyses = futures::future::join_all(image_paths.iter().enumerate().map(|(idx, path)| {
        let provider = &provider;
        let model = &model;
        async move {
            let label = labels
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("[Image #{}]", idx + 1));
            let analysis = match analyze_one_image(state, provider, model, path, user_text).await {
                Ok(text) => text,
                Err(err) => format!("(vision analysis failed: {err})"),
            };
            ImageObservation { label, analysis }
        }
    }))
    .await;

    VisionMixerOutcome::Analyzed {
        provider_name,
        model,
        observations: analyses,
    }
}

/// 纯函数：把视觉观察注入到本轮消息的克隆里。
///
/// 对每条观察追加一条 `system` 消息 `"[Image #N] (vision analysis): <text>"`，置于用户消息**之后**，
/// 使纯文本编码模型在回答前「看到」截图内容。不改动入参 `messages` 以外的任何存储状态——调用方传入的
/// 应当是 `runtime_messages.clone()`（含已 push 的本轮 user 消息）。空观察列表为 no-op。
///
/// 返回注入后的消息向量（按值返回，便于单测对照）。
pub(crate) fn inject_vision_observations(
    mut messages: Vec<Value>,
    observations: &[ImageObservation],
) -> Vec<Value> {
    for obs in observations {
        messages.push(json!({
            "role": "system",
            "content": format!("{} (vision analysis): {}", obs.label, obs.analysis),
        }));
    }
    messages
}

/// 把一批图片读成 inline `image_url` content parts（best-effort）。单张读取失败不致命：
/// 跳过该张并把错误收集进第二个返回值，供调用方提示。供「主编码模型自身支持视觉」时把
/// 图片**直接**塞进主请求（跳过 mixer）用——与 GUI 主模型支持视觉时直发图片的行为对齐。
pub(crate) fn inline_image_parts(image_paths: &[PathBuf]) -> (Vec<Value>, Vec<String>) {
    let mut parts = Vec::new();
    let mut errors = Vec::new();
    for path in image_paths {
        match image_content_part(path) {
            Ok(part) => parts.push(part),
            Err(err) => errors.push(err),
        }
    }
    (parts, errors)
}

/// 纯函数：把 inline 图片 parts 合并进**最后一条 user 消息**的 content，使支持视觉的主模型
/// 直接「看到」截图。图片置于文字之前（与 mixer 的 user 消息约定一致）。无 user 消息或
/// `image_parts` 为空时为 no-op。原 content 为字符串则转成数组（图片 + 文字）；已是数组则
/// 在其前插入图片 parts。调用方应传入 `runtime_messages.clone()`——图片只进本轮克隆，不持久化。
pub(crate) fn inject_inline_images(
    mut messages: Vec<Value>,
    image_parts: Vec<Value>,
) -> Vec<Value> {
    if image_parts.is_empty() {
        return messages;
    }
    let Some(idx) = messages
        .iter()
        .rposition(|m| m.get("role").and_then(Value::as_str) == Some("user"))
    else {
        return messages;
    };
    let mut content = image_parts;
    match messages[idx].get("content").cloned() {
        Some(Value::String(text)) if !text.trim().is_empty() => {
            content.push(json!({ "type": "text", "text": text }));
        }
        Some(Value::Array(existing)) => content.extend(existing),
        _ => {}
    }
    if let Some(obj) = messages[idx].as_object_mut() {
        obj.insert("content".to_string(), Value::Array(content));
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(label: &str, analysis: &str) -> ImageObservation {
        ImageObservation {
            label: label.to_string(),
            analysis: analysis.to_string(),
        }
    }

    #[test]
    fn inject_appends_system_messages_after_user() {
        let base = vec![
            json!({ "role": "system", "content": "sys" }),
            json!({ "role": "user", "content": "look at [Image #1]" }),
        ];
        let observations = vec![obs("[Image #1]", "a red error banner reading E123")];
        let out = inject_vision_observations(base, &observations);

        assert_eq!(out.len(), 3);
        // 用户消息保持不变（占位符 verbatim）。
        assert_eq!(out[1]["content"], json!("look at [Image #1]"));
        // 注入的观察是末尾的一条 system 消息，含占位符 + 分析文本。
        assert_eq!(out[2]["role"], json!("system"));
        let injected = out[2]["content"].as_str().unwrap();
        assert!(injected.starts_with("[Image #1] (vision analysis):"));
        assert!(injected.contains("a red error banner reading E123"));
    }

    #[test]
    fn inject_handles_multiple_images_in_order() {
        let base = vec![json!({ "role": "user", "content": "[Image #1] and [Image #2]" })];
        let observations = vec![obs("[Image #1]", "first"), obs("[Image #2]", "second")];
        let out = inject_vision_observations(base, &observations);

        assert_eq!(out.len(), 3);
        assert!(out[1]["content"].as_str().unwrap().contains("[Image #1]"));
        assert!(out[1]["content"].as_str().unwrap().contains("first"));
        assert!(out[2]["content"].as_str().unwrap().contains("[Image #2]"));
        assert!(out[2]["content"].as_str().unwrap().contains("second"));
    }

    #[test]
    fn inject_empty_is_noop() {
        let base = vec![json!({ "role": "user", "content": "no images here" })];
        let out = inject_vision_observations(base.clone(), &[]);
        assert_eq!(out, base);
    }

    #[test]
    fn inline_empty_parts_is_noop() {
        let base = vec![json!({ "role": "user", "content": "look at this" })];
        assert_eq!(inject_inline_images(base.clone(), vec![]), base);
    }

    #[test]
    fn inline_rewrites_last_user_message_with_image_before_text() {
        let img =
            json!({ "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } });
        let base = vec![
            json!({ "role": "system", "content": "sys" }),
            json!({ "role": "user", "content": "first" }),
            json!({ "role": "assistant", "content": "ok" }),
            json!({ "role": "user", "content": "what is [Image #1]?" }),
        ];
        let out = inject_inline_images(base, vec![img.clone()]);

        // 只改最后一条 user 消息；前面的消息原样保留。
        assert_eq!(out[1]["content"], json!("first"));
        let content = out[3]["content"]
            .as_array()
            .expect("content should be array");
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], json!("image_url")); // 图片在前
        assert_eq!(
            content[1],
            json!({ "type": "text", "text": "what is [Image #1]?" })
        );
    }

    #[test]
    fn inline_handles_empty_text_user_message() {
        let img =
            json!({ "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } });
        let base = vec![json!({ "role": "user", "content": "" })];
        let out = inject_inline_images(base, vec![img]);
        let content = out[0]["content"]
            .as_array()
            .expect("content should be array");
        // 空文字不追加 text part，只剩图片。
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], json!("image_url"));
    }

    #[test]
    fn inline_noop_when_no_user_message() {
        let img = json!({ "type": "image_url", "image_url": { "url": "x" } });
        let base = vec![json!({ "role": "system", "content": "sys" })];
        assert_eq!(inject_inline_images(base.clone(), vec![img]), base);
    }

    #[test]
    fn mime_for_path_covers_image_extensions() {
        assert_eq!(image_mime_for_path(Path::new("a.png")), "image/png");
        assert_eq!(image_mime_for_path(Path::new("a.jpg")), "image/jpeg");
        assert_eq!(image_mime_for_path(Path::new("a.jpeg")), "image/jpeg");
        assert_eq!(image_mime_for_path(Path::new("a.gif")), "image/gif");
        assert_eq!(image_mime_for_path(Path::new("a.webp")), "image/webp");
        // 未知扩展名退回 png（占位符路径不应出现，但保守处理）。
        assert_eq!(image_mime_for_path(Path::new("a.bin")), "image/png");
    }
}
