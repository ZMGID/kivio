//! Shared protocol dispatch after platform credential resolution.
use super::*;
/// 官方 DeepSeek 开内置搜索时的协议跳转：
/// - Chat Completions → Responses 的服务端 `web_search`
/// - 地址已是 `/anthropic` 但协议还标成 Chat Completions → Anthropic Messages
/// 协议本身已是 Responses / Anthropic 时返回 None，走下面的正常分发。
fn official_deepseek_builtin_hop(
    provider: &crate::settings::ModelProvider,
    builtin_web_search: bool,
) -> Option<OfficialDeepseekBuiltinHop> {
    use crate::settings::ProviderApiFormat;
    if !builtin_web_search || !crate::utils::is_official_deepseek_api(&provider.base_url) {
        return None;
    }
    if crate::utils::is_official_deepseek_anthropic_api(&provider.base_url) {
        return match provider.api_format_kind() {
            ProviderApiFormat::AnthropicMessages => None,
            _ => Some(OfficialDeepseekBuiltinHop::Anthropic),
        };
    }
    match provider.api_format_kind() {
        ProviderApiFormat::OpenAiChat => Some(OfficialDeepseekBuiltinHop::Responses),
        _ => None,
    }
}

enum OfficialDeepseekBuiltinHop {
    Responses,
    Anthropic,
}

pub async fn generate_resolved(
    state: &dyn super::host::ModelHost,
    provider: &crate::settings::ModelProvider,
    retry_attempts: usize,
    request: GenerateRequest,
) -> Result<GenerateOutput, ModelError> {
    crate::chat::video::validate_request(provider, &request)?;
    use crate::settings::ProviderApiFormat;
    match official_deepseek_builtin_hop(provider, request.options.builtin_web_search) {
        Some(OfficialDeepseekBuiltinHop::Responses) => {
            return OpenAiResponsesProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await;
        }
        Some(OfficialDeepseekBuiltinHop::Anthropic) => {
            return AnthropicMessagesProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await;
        }
        None => {}
    }
    match provider.api_format_kind() {
        ProviderApiFormat::OpenAiChat => {
            OpenAiChatProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await
        }
        ProviderApiFormat::AnthropicMessages => {
            AnthropicMessagesProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await
        }
        // xAI 与 OpenAI 的 Responses 是同一条线协议，共用适配器；差异只在请求体清洗，
        // 由 `responses.rs` 内部按 `api_format_kind()` 分叉。
        ProviderApiFormat::OpenAiResponses | ProviderApiFormat::XaiResponses => {
            OpenAiResponsesProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await
        }
        ProviderApiFormat::Gemini => {
            GeminiProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await
        }
    }
}

/// `generate_resolved` 的流式版本。同为全 crate 统一分发入口。
pub async fn stream_resolved(
    state: &dyn super::host::ModelHost,
    provider: &crate::settings::ModelProvider,
    retry_attempts: usize,
    request: GenerateRequest,
    sink: &mut (dyn StreamSink + Send),
) -> Result<GenerateOutput, ModelError> {
    crate::chat::video::validate_request(provider, &request)?;
    use crate::settings::ProviderApiFormat;
    match official_deepseek_builtin_hop(provider, request.options.builtin_web_search) {
        Some(OfficialDeepseekBuiltinHop::Responses) => {
            return OpenAiResponsesProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await;
        }
        Some(OfficialDeepseekBuiltinHop::Anthropic) => {
            return AnthropicMessagesProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await;
        }
        None => {}
    }
    match provider.api_format_kind() {
        ProviderApiFormat::OpenAiChat => {
            OpenAiChatProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await
        }
        ProviderApiFormat::AnthropicMessages => {
            AnthropicMessagesProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await
        }
        // xAI 与 OpenAI 的 Responses 是同一条线协议，共用适配器；差异只在请求体清洗，
        // 由 `responses.rs` 内部按 `api_format_kind()` 分叉。
        ProviderApiFormat::OpenAiResponses | ProviderApiFormat::XaiResponses => {
            OpenAiResponsesProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await
        }
        ProviderApiFormat::Gemini => {
            GeminiProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await
        }
    }
}
