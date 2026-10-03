//! 供应商级「请求配置」的请求头装配。
//!
//! 这是自定义头 + CLI 身份头的**唯一装配入口**：发送路径（`apply`）与请求调试面板
//! （`header_pairs`）共用同一个函数，杜绝「面板显示的和实际发的不一致」。写法对齐
//! `chat/model/openai.rs::session_header_pairs` 的既有约定。
//!
//! 校验在这里也做一遍（前端已拦过一层）：settings.json 是用户可以手改的文件，
//! 非法头名会让 reqwest 构造失败，头值里的 CR/LF 是 header 注入。

use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use crate::settings::{ModelProvider, ProviderCustomHeader};

/// 由 Kivio 自己管理、不允许用户覆盖的头。放开会让鉴权/路由错乱。
const RESERVED_HEADER_KEYS: &[&str] = &[
    "authorization",
    "x-api-key",
    "x-goog-api-key",
    "host",
    "content-length",
    "content-encoding",
    "content-type",
    // 适配器已经发了 `Accept-Encoding: identity`，而 reqwest 的 `.header()` 是 append 不是
    // 覆盖 —— 用户再填一条 gzip 会变成 `identity, gzip`，客户端没开 gzip 解码，
    // 认这条头的供应商回来的 SSE 就是一堆二进制垃圾。
    "accept-encoding",
    "anthropic-version",
];

// 内置 CLI 版本号。手填版本为空时用它们。
pub const CLAUDE_CODE_BUILTIN_VERSION: &str = "2.1.287";
pub const CODEX_BUILTIN_VERSION: &str = "0.160.0";
pub const GROK_BUILTIN_VERSION: &str = "0.2.110";
// Claude Code 2.1.x 自带的 Anthropic TS SDK 与 Node 版本（X-Stainless-* 指纹）。
const CLAUDE_CODE_SDK_VERSION: &str = "0.112.1";
const CLAUDE_CODE_NODE_VERSION: &str = "v26.3.0";

/// Claude Code 请求的 system 第一块固定是这句；只换头不换体，网关一眼就能看出是伪装。
pub const CLAUDE_CODE_SYSTEM_PREFIX: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";

pub fn is_claude_code_identity(provider: &ModelProvider) -> bool {
    provider.request.cli_identity.trim() == "claude_code"
}

/// 本机稳定的匿名设备 id（64 位 hex），用于 Anthropic `metadata.user_id`。
/// 首次生成后落盘到 app data，之后每次启动都一样 —— 每次都变的 user_id 本身就是异常信号。
/// 落盘失败只退化成进程内稳定，不影响请求。
pub fn device_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let fresh = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        if cfg!(test) {
            return fresh;
        }
        let Some(dir) = crate::app_data::app_data_dir() else {
            return fresh;
        };
        let path = dir.join("device_id");
        if let Ok(saved) = std::fs::read_to_string(&path) {
            let saved = saved.trim();
            if saved.len() == 64 && saved.bytes().all(|b| b.is_ascii_hexdigit()) {
                return saved.to_string();
            }
        }
        let _ = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &fresh));
        fresh
    })
}

/// 会话 UUID：同一个会话始终映射到同一个 UUID（Kivio 的会话 id 是 `conv_*`，不是 UUID 形状）；
/// 没有会话的一次性调用共用一个进程级 UUID，对齐 Claude Code「一个进程一个会话」。
pub fn session_uuid(conversation_id: Option<&str>) -> String {
    static PROCESS: OnceLock<String> = OnceLock::new();
    match conversation_id.filter(|id| !id.is_empty()) {
        Some(id) => {
            let digest = Sha256::digest(format!("kivio-session:{id}").as_bytes());
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(&digest[..16]);
            uuid::Builder::from_random_bytes(bytes)
                .into_uuid()
                .to_string()
        }
        None => PROCESS
            .get_or_init(|| uuid::Uuid::new_v4().to_string())
            .clone(),
    }
}

/// Codex CLI UA 里的系统段，如 `Mac OS 15.5.0; arm64`（对齐 codex 的 os_info 写法）。
/// 报本机真实值：写死成 Ubuntu 却在 Mac 上跑，本身就是破绽。
fn codex_os_segment() -> String {
    static SEGMENT: OnceLock<String> = OnceLock::new();
    SEGMENT
        .get_or_init(|| {
            let (os, arch) = match std::env::consts::OS {
                "macos" => (
                    format!("Mac OS {}", macos_version().unwrap_or("15.5.0".into())),
                    stainless_arch(),
                ),
                "windows" => ("Windows 10.0.26100".to_string(), std::env::consts::ARCH),
                "linux" => ("Linux".to_string(), std::env::consts::ARCH),
                other => (other.to_string(), std::env::consts::ARCH),
            };
            format!("{os}; {arch}")
        })
        .clone()
}

/// macOS 系统版本，补齐成三段（`15.5` → `15.5.0`）。读不到返回 None。
fn macos_version() -> Option<String> {
    let plist = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist").ok()?;
    let after = plist.split("<key>ProductVersion</key>").nth(1)?;
    let version = after
        .split("<string>")
        .nth(1)?
        .split("</string>")
        .next()?
        .trim();
    if version.is_empty() || !is_valid_header_value(version) {
        return None;
    }
    Some(match version.matches('.').count() {
        0 => format!("{version}.0.0"),
        1 => format!("{version}.0"),
        _ => version.to_string(),
    })
}

/// Codex CLI UA 末尾的终端段：GUI 进程没有终端，取各系统自带终端的写法。
fn codex_terminal() -> &'static str {
    match std::env::consts::OS {
        "macos" => "Apple_Terminal/455.1",
        "windows" => "WindowsTerminal",
        _ => "xterm-256color",
    }
}

/// Stainless SDK 的 OS / 架构写法；报真实主机值，Windows 上报 MacOS 本身就是破绽。
fn stainless_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "MacOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

fn stainless_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        "x86" => "x32",
        other => other,
    }
}

/// RFC 7230 token 字符集。
pub fn is_valid_header_key(key: &str) -> bool {
    !key.is_empty()
        && key.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// 只允许可见 ASCII 与水平制表符：CR/LF 是 header 注入，非 ASCII 会让部分网关直接 400。
pub fn is_valid_header_value(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b == b'\t' || (0x20..=0x7e).contains(&b))
}

pub fn is_reserved_header_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    RESERVED_HEADER_KEYS.contains(&normalized.as_str())
}

/// 一条自定义头是否可用（合法且非保留）。
pub fn is_usable_header(header: &ProviderCustomHeader) -> bool {
    is_valid_header_key(&header.key)
        && is_valid_header_value(&header.value)
        && !is_reserved_header_key(&header.key)
}

/// 版本号会被拼进 User-Agent，所以必须先过头值校验：含 CR/LF 会让 reqwest 构造请求
/// 直接失败（该供应商所有请求报一个看不懂的错），非 ASCII 会被网关 400。非法就退回内置值。
fn identity_version(provider: &ModelProvider, builtin: &str) -> String {
    let configured = provider.request.cli_identity_version.trim();
    if configured.is_empty() || !is_valid_header_value(configured) {
        builtin.to_string()
    } else {
        configured.to_string()
    }
}

/// CLI 身份预设头。命中的网关按 User-Agent 判断客户端类型，只放行特定 CLI。
fn identity_pairs(
    provider: &ModelProvider,
    conversation_id: Option<&str>,
) -> Vec<(String, String)> {
    let p = |k: &str, v: String| (k.to_string(), v);
    match provider.request.cli_identity.trim() {
        "claude_code" => {
            let version = identity_version(provider, CLAUDE_CODE_BUILTIN_VERSION);
            // 整套一起发：按 UA 放行的网关往往同时校验这组 X-Stainless-* 指纹，
            // 少发几条等于没伪装。Content-Type / anthropic-version 由适配器自己带。
            vec![
                p(
                    "User-Agent",
                    format!("claude-cli/{version} (external, cli)"),
                ),
                p("x-app", "cli".to_string()),
                p("X-Stainless-OS", stainless_os().to_string()),
                p("X-Stainless-Arch", stainless_arch().to_string()),
                p("X-Stainless-Lang", "js".to_string()),
                p("X-Stainless-Runtime", "node".to_string()),
                p(
                    "X-Stainless-Runtime-Version",
                    CLAUDE_CODE_NODE_VERSION.to_string(),
                ),
                p(
                    "X-Stainless-Package-Version",
                    CLAUDE_CODE_SDK_VERSION.to_string(),
                ),
                p("X-Stainless-Timeout", "600".to_string()),
                p("X-Stainless-Retry-Count", "0".to_string()),
                p(
                    "anthropic-dangerous-direct-browser-access",
                    "true".to_string(),
                ),
                p("X-Claude-Code-Session-Id", session_uuid(conversation_id)),
            ]
        }
        "codex" => {
            let version = identity_version(provider, CODEX_BUILTIN_VERSION);
            let mut pairs = vec![
                p(
                    "User-Agent",
                    format!(
                        "codex_cli_rs/{version} ({}) {}",
                        codex_os_segment(),
                        codex_terminal()
                    ),
                ),
                // 真实 Codex CLI 每个请求都带 originator + version，只换 UA 等于没伪装。
                p("originator", "codex_cli_rs".to_string()),
                p("version", version),
            ];
            // session_id / conversation_id 是 Codex CLI 链路的会话身份头，真实值是 UUID（且与
            // 请求体 prompt_cache_key 相同）；`conv_*` 形状一看就不是 Codex 发的。没有会话 id 就
            // 不发，编不出来的假 id 只会让会话亲和型网关串台。
            if conversation_id.is_some_and(|id| !id.is_empty()) {
                let id = session_uuid(conversation_id);
                pairs.push(p("session_id", id.clone()));
                pairs.push(p("conversation_id", id));
            }
            pairs
        }
        "grok" => {
            let version = identity_version(provider, GROK_BUILTIN_VERSION);
            vec![p(
                "User-Agent",
                format!("grok-shell/{version} (linux; x86_64)"),
            )]
        }
        _ => Vec::new(),
    }
}

/// 该供应商本次请求要附加的全部头：先铺 CLI 身份预设，再叠用户自定义头（同名覆盖）。
pub fn header_pairs(
    provider: &ModelProvider,
    conversation_id: Option<&str>,
) -> Vec<(String, String)> {
    let mut pairs = identity_pairs(provider, conversation_id);
    for header in &provider.request.custom_headers {
        if !is_usable_header(header) {
            continue;
        }
        // 同名覆盖，并改用用户自己写的大小写（HTTP 头名大小写不敏感，但发出去的
        // 应该是用户填的那个样子）。
        upsert_pair(&mut pairs, header.key.clone(), header.value.clone());
    }
    let oauth = crate::provider_oauth::header_pairs(provider);
    if crate::provider_oauth::is_codex(provider) {
        // Codex OAuth 账号以它自己的 originator / UA 为准（走官方后端、按注册的客户端校验）；
        // 身份预设里配套的 `version` 留着就和 `originator: kivio` 对不上了。
        pairs.retain(|(name, _)| !name.eq_ignore_ascii_case("version"));
    }
    for (name, value) in oauth {
        upsert_pair(&mut pairs, name, value);
    }
    if provider.is_opencode_free() {
        if !pairs
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
        {
            pairs.push((
                "User-Agent".into(),
                concat!("Kivio/", env!("CARGO_PKG_VERSION")).into(),
            ));
        }
        pairs.retain(|(name, _)| {
            !name.eq_ignore_ascii_case("authorization") && !name.eq_ignore_ascii_case("x-api-key")
        });
        if let Some(id) = conversation_id.filter(|id| !id.is_empty()) {
            upsert_pair(&mut pairs, "x-opencode-session".into(), id.into());
        }
    }
    pairs
}

pub fn is_codex_identity(provider: &ModelProvider) -> bool {
    provider.request.cli_identity.trim() == "codex"
}

/// 模型对话适配器（Messages / Chat Completions / Responses）要附加的全部头，发送路径与
/// 请求调试面板共用：
/// 会话亲和头（可选）→ CLI 身份 / 自定义 / OAuth 头（`header_pairs`，同名覆盖）→ 缺省 UA / Accept。
///
/// 缺省头只在前面都没给时才补（reqwest 的 `.header()` 是追加，同名两行会让网关困惑）：
/// reqwest 默认不带 UA，空 UA 本身就像脚本，所以没有身份也没有自定义 UA 时如实报 Kivio；
/// Accept 按请求体是否流式取值（官方 SDK 与 Codex CLI 都是 SSE 发 `text/event-stream`），
/// 由调用方从最终请求体读出 `stream`，体里强制流式（如 Codex OAuth）时头也跟着对。
pub fn model_header_pairs(
    provider: &ModelProvider,
    conversation_id: Option<&str>,
    session_affinity: bool,
    stream: bool,
) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    // 会话亲和头（对齐 opencode）：同一对话每轮带同一 id，会话亲和型代理据此稳定路由到同一
    // 上游会话；正经 provider 忽略未知头。
    if session_affinity {
        if let Some(id) = conversation_id.filter(|id| !id.is_empty()) {
            pairs.push(("x-session-id".into(), id.into()));
            pairs.push(("x-session-affinity".into(), id.into()));
        }
    }
    for (name, value) in header_pairs(provider, conversation_id) {
        upsert_pair(&mut pairs, name, value);
    }
    let accept = if stream {
        "text/event-stream"
    } else {
        "application/json"
    };
    for (name, value) in [
        ("User-Agent", concat!("Kivio/", env!("CARGO_PKG_VERSION"))),
        ("Accept", accept),
    ] {
        if !pairs.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
            pairs.push((name.to_string(), value.to_string()));
        }
    }
    pairs
}

/// 把一条头并进 pairs：同名（大小写不敏感）就整条替换，否则追加。
///
/// reqwest 的 `RequestBuilder::header` 是 **append 不是覆盖**，所以任何「同名只发一条」的
/// 保证都必须在这里做完 —— 一旦两条同名的进了 pairs，上游会收到两行，而请求调试面板用的是
/// BTreeMap，只显示后写的那条，「面板显示的和实际发的」就对不上了。
pub fn upsert_pair(pairs: &mut Vec<(String, String)>, name: String, value: String) {
    match pairs
        .iter_mut()
        .find(|(n, _)| n.eq_ignore_ascii_case(&name))
    {
        Some(existing) => *existing = (name, value),
        None => pairs.push((name, value)),
    }
}

/// 把 `header_pairs` 的结果贴到请求上。
pub fn apply(
    request: reqwest::RequestBuilder,
    provider: &ModelProvider,
    conversation_id: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut request = request;
    for (name, value) in header_pairs(provider, conversation_id) {
        request = request.header(name, value);
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ProviderRequestConfig;

    #[test]
    fn opencode_free_omits_auth_and_preserves_session_affinity() {
        let mut provider = provider_with(ProviderRequestConfig {
            custom_headers: vec![header("Authorization", "do-not-send")],
            ..Default::default()
        });
        provider.base_url = "https://opencode.ai/zen/v1".into();
        provider.api_keys.clear();
        assert!(provider.has_credentials());
        let pairs = header_pairs(&provider, Some("conversation-1"));
        assert!(!pairs
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization")));
        assert!(pairs.contains(&("x-opencode-session".into(), "conversation-1".into())));
        provider.base_url = "https://opencode.ai/zen/go/v1".into();
        assert!(!provider.has_credentials());
    }

    fn provider_with(request: ProviderRequestConfig) -> ModelProvider {
        ModelProvider {
            id: "p".to_string(),
            name: "P".to_string(),
            api_keys: vec!["k".to_string()],
            api_key_legacy: None,
            base_url: "https://x/v1".to_string(),
            available_models: Vec::new(),
            enabled_models: Vec::new(),
            enabled: true,
            api_format: "openai_chat".to_string(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request,
            active_key_index: 0,
        }
    }

    fn header(key: &str, value: &str) -> ProviderCustomHeader {
        ProviderCustomHeader {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn no_config_sends_nothing_extra() {
        let provider = provider_with(ProviderRequestConfig::default());
        assert!(header_pairs(&provider, Some("conv")).is_empty());
    }

    #[test]
    fn custom_headers_pass_through_and_reserved_are_dropped() {
        let provider = provider_with(ProviderRequestConfig {
            custom_headers: vec![
                header("X-Title", "kivio"),
                header("Authorization", "Bearer stolen"),
                header("x-api-key", "nope"),
            ],
            ..Default::default()
        });
        let pairs = header_pairs(&provider, None);
        assert_eq!(pairs, vec![("X-Title".to_string(), "kivio".to_string())]);
    }

    #[test]
    fn malformed_headers_are_dropped() {
        let provider = provider_with(ProviderRequestConfig {
            custom_headers: vec![
                header("Bad Name", "v"),         // 头名里有空格
                header("X-Inject", "a\r\nB: c"), // CRLF 注入
                header("X-Cjk", "中文"),         // 非 ASCII
                header("X-Ok", "fine"),
            ],
            ..Default::default()
        });
        let pairs = header_pairs(&provider, None);
        assert_eq!(pairs, vec![("X-Ok".to_string(), "fine".to_string())]);
    }

    #[test]
    fn custom_header_overrides_identity_preset_case_insensitively() {
        let provider = provider_with(ProviderRequestConfig {
            cli_identity: "claude_code".to_string(),
            custom_headers: vec![header("user-agent", "mine/1.0")],
            ..Default::default()
        });
        let pairs = header_pairs(&provider, None);
        // 覆盖而不是追加：同名头只能有一条，否则上游看到的是哪条全凭运气。
        let uas: Vec<_> = pairs
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
            .collect();
        assert_eq!(uas.len(), 1);
        assert_eq!(uas[0].1, "mine/1.0");
    }

    #[test]
    fn claude_identity_uses_builtin_version_when_unset() {
        let provider = provider_with(ProviderRequestConfig {
            cli_identity: "claude_code".to_string(),
            ..Default::default()
        });
        let pairs = header_pairs(&provider, None);
        assert!(pairs.contains(&(
            "User-Agent".to_string(),
            format!("claude-cli/{CLAUDE_CODE_BUILTIN_VERSION} (external, cli)")
        )));
    }

    #[test]
    fn adapter_owned_headers_cannot_be_overridden() {
        // reqwest 的 .header() 是 append，用户填这几条会变成两行共存：
        // Accept-Encoding 多出 gzip 会让 SSE 流变二进制垃圾，Content-Type 多一条网关 400。
        let provider = provider_with(ProviderRequestConfig {
            custom_headers: vec![
                header("Accept-Encoding", "gzip"),
                header("content-type", "text/plain"),
                header("X-Ok", "1"),
            ],
            ..Default::default()
        });
        assert_eq!(
            header_pairs(&provider, None),
            vec![("X-Ok".to_string(), "1".to_string())]
        );
    }

    #[test]
    fn bad_identity_version_falls_back_to_builtin() {
        // 版本号会被拼进 User-Agent：CR/LF 会让 reqwest 构造请求直接失败（该供应商所有
        // 请求报一个看不懂的错），非 ASCII 会被网关 400。
        const CRLF_VERSION: &str = "2.0\r\n0";
        const TRAILING_LF: &str = "2.0\n";
        let ua_for = |version: &str| {
            let provider = provider_with(ProviderRequestConfig {
                cli_identity: "claude_code".to_string(),
                cli_identity_version: version.to_string(),
                ..Default::default()
            });
            header_pairs(&provider, None)
                .into_iter()
                .find(|(name, _)| name == "User-Agent")
                .map(|(_, value)| value)
                .expect("User-Agent present")
        };

        // 内嵌 CR/LF 与非 ASCII：退回内置版本。
        for bad in [CRLF_VERSION, "中文"] {
            let ua = ua_for(bad);
            assert_eq!(
                ua,
                format!("claude-cli/{CLAUDE_CODE_BUILTIN_VERSION} (external, cli)")
            );
        }
        // 首尾空白（粘贴常带的尾换行）先被 trim 掉，剩下的合法就照用，不必退回。
        assert_eq!(ua_for(TRAILING_LF), "claude-cli/2.0 (external, cli)");

        // 不管走哪条分支，发出去的头值都必须是合法的。
        for version in [CRLF_VERSION, "中文", TRAILING_LF, "", "2.1.71"] {
            assert!(
                is_valid_header_value(&ua_for(version)),
                "version: {version:?}"
            );
        }
    }

    #[test]
    fn claude_identity_sends_the_whole_stainless_fingerprint() {
        // 按 UA 放行的网关往往同时校验这一整组头，少发几条等于没伪装。
        let provider = provider_with(ProviderRequestConfig {
            cli_identity: "claude_code".to_string(),
            ..Default::default()
        });
        let names: Vec<String> = header_pairs(&provider, None)
            .into_iter()
            .map(|(name, _)| name.to_ascii_lowercase())
            .collect();
        for expected in [
            "user-agent",
            "x-app",
            "x-stainless-os",
            "x-stainless-arch",
            "x-stainless-lang",
            "x-stainless-runtime",
            "x-stainless-runtime-version",
            "x-stainless-package-version",
            "x-stainless-timeout",
            "x-stainless-retry-count",
            "anthropic-dangerous-direct-browser-access",
            "x-claude-code-session-id",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing {expected}: {names:?}"
            );
        }
    }

    #[test]
    fn model_headers_default_ua_accept_and_session_affinity() {
        let plain = provider_with(ProviderRequestConfig::default());
        let pairs = model_header_pairs(&plain, Some("conv_1"), true, false);
        assert!(pairs.contains(&("x-session-id".into(), "conv_1".into())));
        assert!(pairs.contains(&("x-session-affinity".into(), "conv_1".into())));
        assert!(pairs.contains(&(
            "User-Agent".into(),
            concat!("Kivio/", env!("CARGO_PKG_VERSION")).into()
        )));
        assert!(pairs.contains(&("Accept".into(), "application/json".into())));
        assert!(model_header_pairs(&plain, None, false, true)
            .contains(&("Accept".into(), "text/event-stream".into())));
        // 不要会话亲和或没有会话：不发亲和头。
        assert!(!model_header_pairs(&plain, Some("conv_1"), false, false)
            .iter()
            .any(|(k, _)| k == "x-session-id"));
        assert!(!model_header_pairs(&plain, None, true, false)
            .iter()
            .any(|(k, _)| k == "x-session-id"));

        // 身份 UA / 用户自定义头优先，且同名只留一条。
        let custom = provider_with(ProviderRequestConfig {
            cli_identity: "codex".into(),
            custom_headers: vec![
                header("accept", "text/event-stream"),
                header("X-Session-Id", "mine"),
            ],
            ..Default::default()
        });
        let pairs = model_header_pairs(&custom, Some("conv_1"), true, true);
        for name in ["user-agent", "accept", "x-session-id"] {
            assert_eq!(
                pairs
                    .iter()
                    .filter(|(k, _)| k.eq_ignore_ascii_case(name))
                    .count(),
                1,
                "{name}: {pairs:?}"
            );
        }
        assert!(pairs.contains(&("accept".into(), "text/event-stream".into())));
        assert!(pairs.contains(&("X-Session-Id".into(), "mine".into())));
        assert!(pairs
            .iter()
            .any(|(k, v)| k == "User-Agent" && v.starts_with("codex_cli_rs/")));
    }

    #[test]
    fn session_uuid_is_stable_per_conversation() {
        let a = session_uuid(Some("conv_a"));
        assert_eq!(a, session_uuid(Some("conv_a")));
        assert_ne!(a, session_uuid(Some("conv_b")));
        assert!(uuid::Uuid::parse_str(&a).is_ok());
        // 一次性调用共用进程级会话。
        assert_eq!(session_uuid(None), session_uuid(Some("")));
    }

    #[test]
    fn codex_identity_session_headers_need_a_conversation_id() {
        let provider = provider_with(ProviderRequestConfig {
            cli_identity: "codex".to_string(),
            cli_identity_version: "1.2.3".to_string(),
            ..Default::default()
        });
        let with_id = header_pairs(&provider, Some("conv-1"));
        // 会话头是 UUID（与请求体 prompt_cache_key 同值），不是 `conv-*`。
        let session = session_uuid(Some("conv-1"));
        assert!(with_id.contains(&("session_id".to_string(), session.clone())));
        assert!(with_id.contains(&("conversation_id".to_string(), session)));
        assert!(with_id.contains(&("originator".to_string(), "codex_cli_rs".to_string())));
        assert!(with_id.contains(&("version".to_string(), "1.2.3".to_string())));
        let ua = &with_id
            .iter()
            .find(|(k, _)| k == "User-Agent")
            .expect("ua")
            .1;
        assert!(ua.starts_with("codex_cli_rs/1.2.3 ("), "{ua}");
        // 报本机真实系统，不再写死 Ubuntu + WindowsTerminal。
        assert!(!ua.contains("Ubuntu"), "{ua}");
        assert!(is_valid_header_value(ua), "{ua}");

        let without_id = header_pairs(&provider, None);
        assert!(!without_id.iter().any(|(k, _)| k == "session_id"));
    }
}
