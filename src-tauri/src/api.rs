//! HTTP 客户端、provider 凭据解析、retry / failover、OpenAI 兼容 chat completion 调用与 SSE 流。
//!
//! 本模块对外暴露：
//! - `ProviderConnectionInput` / `resolve_provider_credentials` —— 来自前端的 provider 临时配置或 settings.json 的解析。
//! - `build_http_client` —— 60s 超时的 reqwest Client 构造。
//! - `effective_retry_attempts` —— 把 settings.retry_enabled + retry_attempts 折成实际尝试次数。
//! - `extract_status_code` / `is_failover_error` —— failover 判定（仅 401/402/403/429）。
//! - `send_with_retry` —— 网络抖动 / 5xx / 429 退避重试。
//! - `send_with_failover` —— 在 api_keys 列表上轮换。
//! - `call_openai_text` / `call_openai_ocr` / `call_vision_api` —— chat completion 三类调用。
//! - `stream_chat_call` / `stream_translate_combined` / `stream_vision_response` —— SSE 流解析。
//! - `build_ocr_request_body` —— 视觉 + 流式 body 构造。

use std::{
  collections::HashSet,
  fs,
  future::Future,
  path::Path,
  sync::atomic::{AtomicU64, Ordering},
  time::Duration,
};

use base64::{engine::general_purpose, Engine as _};
use reqwest::{header::HeaderMap, Client, StatusCode};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::apple_intelligence::APPLE_INTELLIGENCE_BASE_URL;
use crate::prompts::COMBINED_TRANSLATE_SEPARATOR;
use crate::resolve_explain_image_path;
use crate::settings::{
  self, default_system_prompt, no_think_instruction, ExplainMessage, Settings,
};
use crate::state::AppState;

// ===== Provider 凭据 =====

/// 供应商连接输入参数，用于测试连接或获取模型列表时临时传入
/// api_keys 优先；api_key 为兼容旧前端发的单 key 字段（v2.3.x 时的 ProviderConnectionInput）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionInput {
  pub id: Option<String>,
  pub base_url: String,
  #[serde(default)]
  pub api_keys: Vec<String>,
  #[serde(default)]
  pub api_key: Option<String>,
}

impl ProviderConnectionInput {
  /// 整理出非空 key 列表：优先 api_keys，回退到 api_key。
  pub fn merged_keys(&self) -> Vec<String> {
    let mut keys: Vec<String> = self
      .api_keys
      .iter()
      .map(|k| k.trim().to_string())
      .filter(|k| !k.is_empty())
      .collect();
    if keys.is_empty() {
      if let Some(legacy) = self.api_key.as_deref() {
        let trimmed = legacy.trim().to_string();
        if !trimmed.is_empty() {
          keys.push(trimmed);
        }
      }
    }
    keys
  }
}

/// 解析供应商的凭据信息（base_url + 多 key 列表）
/// 优先使用传入的 ProviderConnectionInput（如测试连接时），否则从 settings 中查找对应的供应商
pub fn resolve_provider_credentials(
  settings: &Settings,
  provider_id: &str,
  provider: Option<ProviderConnectionInput>,
) -> Result<(String, Vec<String>), String> {
  if let Some(input) = provider {
    let id_matches = input
      .id
      .as_ref()
      .map(|id| id.is_empty() || id == provider_id)
      .unwrap_or(true);

    if id_matches {
      return Ok((input.base_url.clone(), input.merged_keys()));
    }
  }

  let provider = settings
    .get_provider(provider_id)
    .ok_or_else(|| "Provider not found".to_string())?;
  Ok((provider.base_url.clone(), provider.api_keys.clone()))
}

/// 构建 HTTP 客户端，设置 60 秒超时
pub fn build_http_client() -> Client {
  Client::builder()
    .timeout(Duration::from_secs(60))
    .build()
    .unwrap_or_else(|err| {
      eprintln!("Failed to build HTTP client: {err}");
      Client::new()
    })
}

// ===== Retry / Failover =====

/// 重试延迟基础值（毫秒）
const RETRY_BASE_DELAY_MS: u64 = 500;
/// 重试延迟最大值（毫秒）
const RETRY_MAX_DELAY_MS: u64 = 10_000;

/// 获取实际的重试次数
/// 如果重试功能被禁用，则返回 1（即只尝试一次）
pub fn effective_retry_attempts(settings: &Settings) -> usize {
  if settings.retry_enabled {
    settings.retry_attempts as usize
  } else {
    1
  }
}

/// 从响应头中解析 Retry-After 值（秒），转换为毫秒延迟
fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
  headers
    .get("retry-after")
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.parse::<u64>().ok())
}

/// 判断 HTTP 状态码是否可重试
/// 包括 429（限流）和所有服务器错误（5xx）
fn is_retryable_status(status: StatusCode) -> bool {
  status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// 判断请求错误是否可重试
/// 包括超时和连接错误
fn is_retryable_error(error: &reqwest::Error) -> bool {
  error.is_timeout() || error.is_connect()
}

/// 计算重试延迟
/// 优先使用服务器返回的 Retry-After 头；否则使用指数退避策略
fn retry_delay_ms(attempt: usize, retry_after: Option<u64>) -> u64 {
  if let Some(seconds) = retry_after {
    return seconds.saturating_mul(1000);
  }

  let delay = RETRY_BASE_DELAY_MS.saturating_mul(2u64.saturating_pow((attempt - 1) as u32));
  delay.min(RETRY_MAX_DELAY_MS)
}

/// 从 send_with_retry 拼接的错误信息中提取 HTTP 状态码
/// 格式约定：`"{label} Error: {status} - {body}"`，
/// status 形如 `"429 Too Many Requests"`，第一段数字即可
/// 网络错误（reqwest::Error 路径）格式为 `"{label} Error: <reqwest msg>"`，无前导数字 → 返回 None
pub fn extract_status_code(err_msg: &str) -> Option<u16> {
  let idx = err_msg.find(" Error: ")?;
  let rest = &err_msg[idx + " Error: ".len()..];
  let end = rest
    .find(|c: char| !c.is_ascii_digit())
    .unwrap_or(rest.len());
  if end == 0 {
    return None;
  }
  rest[..end].parse().ok()
}

/// 判断错误信息是否触发 key failover
/// 严格按 HTTP 状态码：401/402/403/429 才换 key —— 与 key 直接相关的错误：
/// - 401 鉴权失败（key 被吊销 / 错误）
/// - 402 需要付费（账户欠费）
/// - 403 权限不足 / 被封禁
/// - 429 限流（key 维度配额耗尽）
/// 其它 4xx（如 400 malformed body）属于请求本身问题，换 key 也无济于事 → 不触发
/// 5xx 由 send_with_retry 内部退避重试，不会到这里
/// 网络错误（timeout / connect 失败）非 key 问题，extract_status_code 返回 None → 不触发
pub fn is_failover_error(err_msg: &str) -> bool {
  matches!(extract_status_code(err_msg), Some(401 | 402 | 403 | 429))
}

/// 多 key failover 包装：在 api_keys 列表上依次尝试，遇到 failover-eligible 错误自动切下一 key
/// 内层每次尝试仍走 send_with_retry（处理网络抖动 / 服务端 5xx 等通用重试）
pub async fn send_with_failover<F, Fut>(
  state: &AppState,
  label: &str,
  attempts: usize,
  provider_id: &str,
  api_keys: &[String],
  send: F,
) -> Result<reqwest::Response, String>
where
  F: Fn(&str) -> Fut,
  Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
  let total = api_keys.len();
  if total == 0 {
    return Err(format!("{} Error: No API key configured", label));
  }

  let mut tried: HashSet<usize> = HashSet::new();
  let mut last_err: Option<String> = None;

  while tried.len() < total {
    let idx = match state.pick_active_key(provider_id, total, &tried) {
      Some(i) => i,
      None => break,
    };
    tried.insert(idx);
    let key = api_keys[idx].as_str();

    match send_with_retry(label, attempts, || send(key)).await {
      Ok(resp) => {
        state.mark_key_ok(provider_id, idx);
        return Ok(resp);
      }
      Err(err_msg) => {
        if is_failover_error(&err_msg) && tried.len() < total {
          state.mark_key_failed(provider_id, idx);
          eprintln!(
            "[failover] {} key #{}/{} failed, switching to next: {}",
            label,
            idx + 1,
            total,
            err_msg
          );
          last_err = Some(err_msg);
          continue;
        }
        // 非 failover 错误（或已穷举所有 key）→ 直接返回
        if is_failover_error(&err_msg) {
          state.mark_key_failed(provider_id, idx);
        }
        return Err(err_msg);
      }
    }
  }

  Err(
    last_err.unwrap_or_else(|| format!("{} Error: all {} keys exhausted", label, total)),
  )
}

/// 带重试机制的 HTTP 发送函数
/// 对可重试的错误（限流、服务器错误、超时、连接失败）进行指数退避重试
pub async fn send_with_retry<F, Fut>(
  label: &str,
  attempts: usize,
  mut send: F,
) -> Result<reqwest::Response, String>
where
  F: FnMut() -> Fut,
  Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
  let attempts = attempts.max(1);
  let mut last_error: Option<String> = None;

  for attempt in 1..=attempts {
    match send().await {
      Ok(response) => {
        let status = response.status();
        if status.is_success() {
          return Ok(response);
        }

        let retry_after = parse_retry_after(response.headers());
        let text = response.text().await.unwrap_or_default();
        let err_msg = format!("{} Error: {} - {}", label, status, text);

        if is_retryable_status(status) && attempt < attempts {
          last_error = Some(err_msg);
          let delay = retry_delay_ms(attempt, retry_after);
          eprintln!("{} retrying in {}ms (attempt {}/{})", label, delay, attempt, attempts);
          tokio::time::sleep(Duration::from_millis(delay)).await;
          continue;
        }

        return Err(format!("{} (attempt {}/{})", err_msg, attempt, attempts));
      }
      Err(err) => {
        let err_msg = format!("{} Error: {}", label, err);
        if is_retryable_error(&err) && attempt < attempts {
          last_error = Some(err_msg);
          let delay = retry_delay_ms(attempt, None);
          eprintln!("{} retrying in {}ms (attempt {}/{})", label, delay, attempt, attempts);
          tokio::time::sleep(Duration::from_millis(delay)).await;
          continue;
        }
        return Err(format!("{} (attempt {}/{})", err_msg, attempt, attempts));
      }
    }
  }

  Err(last_error.map(|msg| {
    format!("{} (attempt {}/{})", msg, attempts, attempts)
  }).unwrap_or_else(|| {
    format!("{} Error: exceeded retry attempts ({})", label, attempts)
  }))
}

// ===== Chat completion 调用 =====

/// 调用 OpenAI 兼容的文本聊天接口
/// 发送单轮 user 消息，temperature 设为 0.2,返回模型生成的文本内容
pub async fn call_openai_text(
  state: &State<'_, AppState>,
  config: &settings::ModelProvider,
  model: &str,
  prompt: String,
  retry_attempts: usize,
  thinking_enabled: bool,
) -> Result<String, String> {
  // Apple Intelligence(端上)路由：跳过 HTTP，直接调 sidecar。model/retry/thinking 三个参数全部忽略。
  if config.base_url == APPLE_INTELLIGENCE_BASE_URL {
    let _ = (model, retry_attempts, thinking_enabled);
    return state.apple_intelligence.call_text(&prompt).await;
  }
  let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
  let mut body = serde_json::json!({
    "model": model,
    "messages": [{ "role": "user", "content": prompt }],
    "temperature": 0.2
  });
  if !thinking_enabled {
    body["thinking"] = serde_json::json!({ "type": "disabled" });
  }

  let response = send_with_failover(
    state,
    "OpenAI API",
    retry_attempts,
    &config.id,
    &config.api_keys,
    |key| {
      state
        .http
        .post(url.clone())
        .bearer_auth(key)
        .json(&body)
        .send()
    },
  )
  .await?;

  let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
  let content = value
    .get("choices")
    .and_then(|choices| choices.get(0))
    .and_then(|choice| choice.get("message"))
    .and_then(|message| message.get("content"))
    .and_then(|content| content.as_str())
    .ok_or_else(|| "Invalid response".to_string())?;

  Ok(content.trim().to_string())
}

/// 调用 OpenAI 兼容的 OCR/视觉接口
/// 将图片转为 Base64 后作为 image_url 类型内容发送，temperature 设为 0 以提高识别稳定性
pub async fn call_openai_ocr(
  state: &State<'_, AppState>,
  config: &settings::ModelProvider,
  model: &str,
  image_path: &Path,
  prompt: &str,
  retry_attempts: usize,
  thinking_enabled: bool,
) -> Result<String, String> {
  if config.base_url == APPLE_INTELLIGENCE_BASE_URL {
    let _ = (state, model, image_path, prompt, retry_attempts, thinking_enabled);
    return Err("Apple Intelligence 暂不支持图像输入,请为截图/视觉功能配置云端 provider".into());
  }
  let bytes = fs::read(image_path).map_err(|e| e.to_string())?;
  let base64 = general_purpose::STANDARD.encode(bytes);
  let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));

  // 与 lens 的 vision body 对齐：image 在 text 前、显式 max_tokens。
  // thinking 按调用方传入：截图翻译默认 false（节省时间），lens 默认 true。
  let mut body = serde_json::json!({
    "model": model,
    "messages": [
      {
        "role": "user",
        "content": [
          {
            "type": "image_url",
            "image_url": { "url": format!("data:image/png;base64,{base64}") }
          },
          {
            "type": "text",
            "text": prompt
          }
        ]
      }
    ],
    "temperature": 0.2,
    "max_tokens": 2000
  });
  if !thinking_enabled {
    body["thinking"] = serde_json::json!({ "type": "disabled" });
  }

  let response = send_with_failover(
    state,
    "OpenAI OCR",
    retry_attempts,
    &config.id,
    &config.api_keys,
    |key| {
      state
        .http
        .post(url.clone())
        .bearer_auth(key)
        .json(&body)
        .send()
    },
  )
  .await?;

  // 显式检查 HTTP 状态：非 2xx 把原始 body 文本带回，避免后续 .json() 抛出含糊的 "error decoding response body"
  let status = response.status();
  if !status.is_success() {
    let body_text = response.text().await.unwrap_or_default();
    let snippet: String = body_text.chars().take(500).collect();
    return Err(format!("OCR HTTP {}: {}", status.as_u16(), snippet));
  }

  let raw = response.text().await.map_err(|e| format!("OCR read body: {}", e))?;
  let value: serde_json::Value = serde_json::from_str(&raw)
    .map_err(|e| format!("OCR parse JSON: {} (body: {})", e, raw.chars().take(500).collect::<String>()))?;
  let content = value
    .get("choices")
    .and_then(|choices| choices.get(0))
    .and_then(|choice| choice.get("message"))
    .and_then(|message| message.get("content"))
    .and_then(|content| content.as_str())
    .ok_or_else(|| format!("Invalid OCR response: {}", raw.chars().take(500).collect::<String>()))?;

  Ok(content.trim().to_string())
}

/// 调用视觉 API（截图解释 / Lens 共用）
/// 支持流式输出：如果 stream 为 true，通过 stream_vision_response 逐段 emit `event_name` 事件。
/// `provider_id_override` 非空时使用指定 provider/model（用于 lens 选择独立模型）；空则走 explain 配置。
#[allow(clippy::too_many_arguments)]
pub async fn call_vision_api(
  app: &AppHandle,
  state: &State<'_, AppState>,
  image_id: &str,
  messages: Vec<ExplainMessage>,
  language: &str,
  retry_attempts: usize,
  stream: bool,
  stream_kind: &str,
  event_name: &str,
  provider_id_override: Option<&str>,
  model_override: Option<&str>,
  system_prompt_override: Option<&str>,
  thinking_enabled: bool,
) -> Result<String, String> {
  let settings = state.settings_read().clone();
  let provider_id = provider_id_override
    .filter(|s| !s.is_empty())
    .unwrap_or(&settings.translator_provider_id);
  let provider = settings.get_provider(provider_id)
    .ok_or_else(|| "Vision provider not found".to_string())?;
  if provider.base_url == APPLE_INTELLIGENCE_BASE_URL {
    return Err("Apple Intelligence 暂不支持图像输入,请为 Lens / 截图视觉功能配置云端 provider".into());
  }

  // image_id 为空 → 走纯文本对话路径（不附图）
  let has_image = !image_id.is_empty();

  let mut api_messages = Vec::new();
  // 优先用调用方传入的 system_prompt_override；否则用默认模板（区分有/无图片）
  // 关闭思考时在 system 末尾追加显式禁止指令，作为参数层不生效时的兜底
  let system_prompt_to_use: Option<String> = {
    let base = match system_prompt_override.filter(|s| !s.is_empty()) {
      Some(s) => s.to_string(),
      None => default_system_prompt(language, has_image),
    };
    if !thinking_enabled {
      Some(format!("{}{}", base, no_think_instruction(language)))
    } else {
      Some(base)
    }
  };
  if let Some(sp) = system_prompt_to_use {
    api_messages.push(serde_json::json!({
      "role": "system",
      "content": sp
    }));
  }

  if has_image {
    let image_path = resolve_explain_image_path(app, state, image_id)?;
    let bytes = fs::read(image_path).map_err(|e| e.to_string())?;
    let base64 = general_purpose::STANDARD.encode(bytes);
    if let Some(first) = messages.first() {
      api_messages.push(serde_json::json!({
        "role": "user",
        "content": [
          { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{base64}") } },
          { "type": "text", "text": first.content }
        ]
      }));
      for message in messages.iter().skip(1) {
        api_messages.push(serde_json::json!({
          "role": message.role,
          "content": message.content
        }));
      }
    }
  } else {
    // 纯文本：每条 message 直接 push（无图）
    for message in messages.iter() {
      api_messages.push(serde_json::json!({
        "role": message.role,
        "content": message.content,
      }));
    }
  }

  let model = model_override
    .filter(|s| !s.is_empty())
    .unwrap_or(&settings.translator_model);
  let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));
  let mut body = serde_json::json!({
    "model": model,
    "messages": api_messages,
    "temperature": 0.7,
    "max_tokens": 2000
  });
  if stream {
    body["stream"] = serde_json::json!(true);
  }

  // 关闭思考模式：仅塞 DeepSeek/Kimi 官方文档约定的 thinking={type:"disabled"} 字段。
  // 不再注入 chat_template_kwargs / enable_thinking / reasoning_effort —— 这些是 vLLM/Qwen/OpenAI
  // 私有字段，第三方代理（如 OpenRouter / 反代）做严格校验时会以 400 拒绝整个请求（实测 DeepSeek
  // 路径上 chat_template_kwargs 直接报错）。提示词层的 no-think 指令是更稳的兜底。
  if !thinking_enabled {
    body["thinking"] = serde_json::json!({ "type": "disabled" });
  }

  let response = send_with_failover(
    state,
    "Vision API",
    retry_attempts,
    &provider.id,
    &provider.api_keys,
    |key| {
      state
        .http
        .post(url.clone())
        .bearer_auth(key)
        .json(&body)
        .send()
    },
  )
  .await?;

  // 先检查 HTTP 状态：非 2xx 直接读出 body 文本作为错误，避免后续 .json() / chunk() 拿到非预期格式时抛出含糊的 "error decoding response body"。
  let status = response.status();
  if !status.is_success() {
    let body_text = response.text().await.unwrap_or_default();
    let snippet = body_text.chars().take(500).collect::<String>();
    return Err(format!("Vision API HTTP {}: {}", status.as_u16(), snippet));
  }

  if stream {
    // 启动新流：递增代号，存到本流持有的快照里；后续 chunk 循环只要发现全局代号 != 自己的快照就退出。
    let generation = state
      .explain_stream_generation
      .fetch_add(1, Ordering::SeqCst)
      + 1;
    return stream_vision_response(
      app,
      response,
      image_id,
      stream_kind,
      event_name,
      &state.explain_stream_generation,
      generation,
    )
    .await;
  }

  // 非流式：先读 raw text，再 parse JSON，把原始 body 作为错误信息便于诊断。
  let raw = response.text().await.map_err(|e| format!("Vision API read body: {}", e))?;
  let value: serde_json::Value = serde_json::from_str(&raw)
    .map_err(|e| format!("Vision API parse JSON: {} (body: {})", e, raw.chars().take(500).collect::<String>()))?;
  let content = value
    .get("choices")
    .and_then(|choices| choices.get(0))
    .and_then(|choice| choice.get("message"))
    .and_then(|message| message.get("content"))
    .and_then(|content| content.as_str())
    .ok_or_else(|| format!("Invalid vision response: {}", raw.chars().take(500).collect::<String>()))?;

  Ok(content.trim().to_string())
}

// ===== SSE 流 =====

/// 构造带 image 的 OCR/视觉请求 body（model 由调用方注入），开启 stream
pub fn build_ocr_request_body(
  image_path: &Path,
  prompt: &str,
  thinking_enabled: bool,
) -> Result<serde_json::Value, String> {
  let bytes = fs::read(image_path).map_err(|e| e.to_string())?;
  let base64 = general_purpose::STANDARD.encode(bytes);
  let mut body = serde_json::json!({
    "messages": [{
      "role": "user",
      "content": [
        { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{base64}") } },
        { "type": "text", "text": prompt }
      ]
    }],
    "temperature": 0.2,
    "max_tokens": 2000,
    "stream": true
  });
  if !thinking_enabled {
    body["thinking"] = serde_json::json!({ "type": "disabled" });
  }
  Ok(body)
}

/// 通用流式 chat 调用：发送 body（model 在外层注入）→ 解析 SSE → 通过 stream_vision_response emit。
/// 复用 explain_stream_generation 作取消代号（lens-stream / lens-translate-stream 都共用）。
#[allow(clippy::too_many_arguments)]
pub async fn stream_chat_call(
  app: &AppHandle,
  state: &State<'_, AppState>,
  provider: &settings::ModelProvider,
  model: &str,
  mut body: serde_json::Value,
  retry_attempts: usize,
  image_id: &str,
  kind: &str,
  event_name: &str,
) -> Result<String, String> {
  if provider.base_url == APPLE_INTELLIGENCE_BASE_URL {
    let _ = (app, state, model, &mut body, retry_attempts, image_id, kind, event_name);
    return Err("Apple Intelligence 暂不支持图像输入,请为截图翻译配置云端 provider".into());
  }
  body["model"] = serde_json::json!(model);
  let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));

  let response = send_with_failover(
    state,
    "Stream chat",
    retry_attempts,
    &provider.id,
    &provider.api_keys,
    |key| {
      state
        .http
        .post(url.clone())
        .bearer_auth(key)
        .json(&body)
        .send()
    },
  )
  .await?;

  let status = response.status();
  if !status.is_success() {
    let body_text = response.text().await.unwrap_or_default();
    let snippet: String = body_text.chars().take(500).collect();
    return Err(format!("Stream HTTP {}: {}", status.as_u16(), snippet));
  }

  let generation = state
    .explain_stream_generation
    .fetch_add(1, Ordering::SeqCst)
    + 1;
  stream_vision_response(
    app,
    response,
    image_id,
    kind,
    event_name,
    &state.explain_stream_generation,
    generation,
  )
  .await
}

/// 截图翻译合并模式流：单次调用模型，按 `<<<ORIGINAL>>>` 分隔符把 SSE delta 拆成两段。
/// 分隔符前的 chunk emit kind="translated"；分隔符后的 chunk emit kind="original"。
/// 返回 (translated, original) 完整文本。
///
/// 关键点：
/// - 分隔符可能跨 SSE chunk 边界 → 用 tail 缓冲住末尾 (SEPARATOR.len()-1) 字节防止把分隔符前缀当成译文 emit 出去
/// - tail 切片必须落在 UTF-8 char boundary，否则 String::drain 会 panic（用户截图常含 CJK，每字 3 字节）
#[allow(clippy::too_many_arguments)]
pub async fn stream_translate_combined(
  app: &AppHandle,
  state: &State<'_, AppState>,
  provider: &settings::ModelProvider,
  model: &str,
  mut body: serde_json::Value,
  retry_attempts: usize,
  image_id: &str,
  event_name: &str,
) -> Result<(String, String), String> {
  if provider.base_url == APPLE_INTELLIGENCE_BASE_URL {
    let _ = (app, state, model, &mut body, retry_attempts, image_id, event_name);
    return Err("Apple Intelligence 暂不支持图像输入,请为截图翻译配置云端 provider".into());
  }
  body["model"] = serde_json::json!(model);
  let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));

  let mut response = send_with_failover(
    state,
    "Stream translate combined",
    retry_attempts,
    &provider.id,
    &provider.api_keys,
    |key| {
      state
        .http
        .post(url.clone())
        .bearer_auth(key)
        .json(&body)
        .send()
    },
  )
  .await?;

  let status = response.status();
  if !status.is_success() {
    let body_text = response.text().await.unwrap_or_default();
    let snippet: String = body_text.chars().take(500).collect();
    return Err(format!("Stream HTTP {}: {}", status.as_u16(), snippet));
  }

  let my_gen = state
    .explain_stream_generation
    .fetch_add(1, Ordering::SeqCst)
    + 1;

  let sep = COMBINED_TRANSLATE_SEPARATOR;
  let sep_len = sep.len();

  let mut sse_buf = String::new();
  let mut tail = String::new();
  let mut translated = String::new();
  let mut original = String::new();
  let mut sep_seen = false;

  let emit_done = |reason: &str| {
    let _ = app.emit(
      event_name,
      serde_json::json!({
        "imageId": image_id, "delta": "", "done": true, "reason": reason,
      }),
    );
  };

  loop {
    if state.explain_stream_generation.load(Ordering::SeqCst) != my_gen {
      // 取消：把 tail 当作 translated flush（避免末尾几个字符丢失），再 emit done
      if !tail.is_empty() && !sep_seen {
        translated.push_str(&tail);
        let _ = app.emit(
          event_name,
          serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": tail }),
        );
      }
      emit_done("cancelled");
      return Ok((translated, original));
    }

    let chunk = match response.chunk().await {
      Ok(Some(c)) => c,
      Ok(None) => break,
      Err(e) => {
        emit_done("error");
        return Err(e.to_string());
      }
    };

    let text = String::from_utf8_lossy(&chunk);
    sse_buf.push_str(&text);

    while let Some(pos) = sse_buf.find('\n') {
      let line: String = sse_buf.drain(..=pos).collect();
      let line = line.trim();
      if !line.starts_with("data:") {
        continue;
      }
      let data = line.trim_start_matches("data:").trim();
      if data.is_empty() {
        continue;
      }
      if data == "[DONE]" {
        // flush tail
        if !sep_seen && !tail.is_empty() {
          translated.push_str(&tail);
          let _ = app.emit(
            event_name,
            serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": tail }),
          );
        }
        emit_done("done");
        return Ok((translated, original));
      }

      let value: serde_json::Value = match serde_json::from_str(data) {
        Ok(val) => val,
        Err(_) => continue,
      };

      let delta_obj = value
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("delta"));

      // 推理链 emit（恒定 kind="translated"，前端在主面板渲染）
      if let Some(r) = delta_obj
        .and_then(|d| d.get("reasoning_content").or_else(|| d.get("reasoning")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
      {
        let _ = app.emit(
          event_name,
          serde_json::json!({
            "imageId": image_id, "kind": "translated", "delta": "", "reasoningDelta": r,
          }),
        );
      }

      let content = delta_obj
        .and_then(|d| d.get("content"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

      let Some(c) = content else { continue };

      if sep_seen {
        original.push_str(c);
        let _ = app.emit(
          event_name,
          serde_json::json!({ "imageId": image_id, "kind": "original", "delta": c }),
        );
        continue;
      }

      tail.push_str(c);
      if let Some(idx) = tail.find(sep) {
        // 分隔符命中：拆 before / after，trim 掉分隔符相邻的换行，分别发出
        let before: String = tail.drain(..idx).collect();
        // 移除分隔符本身
        tail.drain(..sep_len);
        let after: String = std::mem::take(&mut tail);

        let before_emit = before.trim_end_matches('\n').to_string();
        if !before_emit.is_empty() {
          translated.push_str(&before_emit);
          let _ = app.emit(
            event_name,
            serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": before_emit }),
          );
        }
        sep_seen = true;
        let after_emit = after.trim_start_matches('\n').to_string();
        if !after_emit.is_empty() {
          original.push_str(&after_emit);
          let _ = app.emit(
            event_name,
            serde_json::json!({ "imageId": image_id, "kind": "original", "delta": after_emit }),
          );
        }
      } else {
        // 没命中：emit 安全前缀（保留末尾 sep_len-1 字节防止跨 chunk 分隔符被切碎）
        let max_emit = tail.len().saturating_sub(sep_len.saturating_sub(1));
        if max_emit == 0 {
          continue;
        }
        // 找一个合法 char boundary（CJK 字符多字节，不能切到字符中间）
        let mut safe = max_emit;
        while safe > 0 && !tail.is_char_boundary(safe) {
          safe -= 1;
        }
        if safe == 0 {
          continue;
        }
        let to_emit: String = tail.drain(..safe).collect();
        translated.push_str(&to_emit);
        let _ = app.emit(
          event_name,
          serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": to_emit }),
        );
      }
    }
  }

  // SSE 流结束（连接关闭）但没收到 [DONE]：flush tail
  if !sep_seen && !tail.is_empty() {
    translated.push_str(&tail);
    let _ = app.emit(
      event_name,
      serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": tail }),
    );
  }
  emit_done("done");
  Ok((translated, original))
}

/// 流式解析视觉 API 的 SSE 响应
/// 逐 chunk 读取响应体，解析 "data:" 行，提取 delta 中的 content 并通过 `event_name` emit。
/// 支持取消：调用方持有 `my_generation`，全局代号 `generation_atom` 一旦变化即视为被新流或外部取消作废。
pub async fn stream_vision_response(
  app: &AppHandle,
  mut response: reqwest::Response,
  image_id: &str,
  kind: &str,
  event_name: &str,
  generation_atom: &AtomicU64,
  my_generation: u64,
) -> Result<String, String> {
  let mut buffer = String::new();
  let mut full = String::new();

  let emit_done = |reason: &str, full_text: &str| {
    let _ = app.emit(
      event_name,
      serde_json::json!({
        "imageId": image_id,
        "kind": kind,
        "delta": "",
        "done": true,
        "reason": reason,
        "full": full_text,
      }),
    );
  };

  loop {
    if generation_atom.load(Ordering::SeqCst) != my_generation {
      emit_done("cancelled", full.trim());
      return Ok(full.trim().to_string());
    }

    let chunk = match response.chunk().await {
      Ok(Some(c)) => c,
      Ok(None) => break,
      Err(e) => {
        emit_done("error", full.trim());
        return Err(e.to_string());
      }
    };

    let text = String::from_utf8_lossy(&chunk);
    buffer.push_str(&text);

    while let Some(pos) = buffer.find('\n') {
      let line: String = buffer.drain(..=pos).collect();
      let line = line.trim();
      if !line.starts_with("data:") {
        continue;
      }
      let data = line.trim_start_matches("data:").trim();
      if data.is_empty() {
        continue;
      }
      if data == "[DONE]" {
        emit_done("done", full.trim());
        return Ok(full.trim().to_string());
      }

      let value: serde_json::Value = match serde_json::from_str(data) {
        Ok(val) => val,
        Err(_) => continue,
      };

      let delta_obj = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("delta"));

      // 推理模型（DeepSeek-R1 / Kimi 等）把链路放在 delta.reasoning_content
      // 部分实现用 delta.reasoning。两种字段都尝试取，只要有就 emit。
      let reasoning = delta_obj
        .and_then(|d| d.get("reasoning_content").or_else(|| d.get("reasoning")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

      if let Some(r) = reasoning {
        let _ = app.emit(
          event_name,
          serde_json::json!({
            "imageId": image_id,
            "kind": kind,
            "delta": "",
            "reasoningDelta": r,
          }),
        );
      }

      let content = delta_obj
        .and_then(|d| d.get("content"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

      if let Some(content) = content {
        full.push_str(content);
        let _ = app.emit(
          event_name,
          serde_json::json!({ "imageId": image_id, "kind": kind, "delta": content }),
        );
      }
    }
  }

  emit_done("done", full.trim());
  Ok(full.trim().to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  // ===== extract_status_code =====

  #[test]
  fn extract_status_code_parses_typical_send_with_retry_format() {
    // send_with_retry 拼出来的标准格式
    let s = "OpenAI API Error: 429 Too Many Requests - {\"error\":\"rate_limit\"}";
    assert_eq!(extract_status_code(s), Some(429));
  }

  #[test]
  fn extract_status_code_handles_each_failover_status() {
    assert_eq!(extract_status_code("X Error: 401 Unauthorized - body"), Some(401));
    assert_eq!(extract_status_code("X Error: 402 Payment Required - body"), Some(402));
    assert_eq!(extract_status_code("X Error: 403 Forbidden - body"), Some(403));
    assert_eq!(extract_status_code("X Error: 429 Too Many Requests - body"), Some(429));
  }

  #[test]
  fn extract_status_code_handles_non_failover_status() {
    assert_eq!(extract_status_code("X Error: 400 Bad Request - body"), Some(400));
    assert_eq!(extract_status_code("X Error: 500 Internal Server Error - body"), Some(500));
  }

  #[test]
  fn extract_status_code_returns_none_for_network_error() {
    // reqwest::Error 路径无前导数字
    let s = "Stream chat Error: error sending request: connection refused (attempt 3/3)";
    assert_eq!(extract_status_code(s), None);
  }

  #[test]
  fn extract_status_code_returns_none_when_marker_missing() {
    assert_eq!(extract_status_code("just some message"), None);
    assert_eq!(extract_status_code(""), None);
  }

  // ===== is_failover_error =====

  #[test]
  fn is_failover_error_only_triggers_on_auth_quota_codes() {
    assert!(is_failover_error("X Error: 401 - body"));
    assert!(is_failover_error("X Error: 402 - body"));
    assert!(is_failover_error("X Error: 403 - body"));
    assert!(is_failover_error("X Error: 429 - body"));
  }

  #[test]
  fn is_failover_error_does_not_trigger_on_400_or_5xx() {
    // 400 是请求 body 问题，不应换 key
    assert!(!is_failover_error("X Error: 400 Bad Request - body"));
    // 500 由 send_with_retry 内部退避重试，不应到 failover 层
    assert!(!is_failover_error("X Error: 500 Internal Server Error - body"));
    assert!(!is_failover_error("X Error: 503 Service Unavailable - body"));
  }

  #[test]
  fn is_failover_error_does_not_trigger_on_network_failure() {
    // 网络问题不是 key 的锅
    assert!(!is_failover_error("Stream Error: error sending request: timed out"));
    assert!(!is_failover_error("X Error: connection closed"));
  }

  #[test]
  fn is_failover_error_does_not_trigger_on_body_keywords_alone() {
    // 旧版宽泛匹配 body 含 "billing" / "quota" 会误触发；现版严格按状态码
    assert!(!is_failover_error("X Error: 400 - {\"message\":\"billing issue\"}"));
    assert!(!is_failover_error("X Error: 500 - {\"message\":\"quota exceeded\"}"));
  }
}
