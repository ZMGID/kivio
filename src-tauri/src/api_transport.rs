//! Shared HTTP request construction, retry classification and stream decoding.
//!
//! This source has no desktop-host or application-state dependency. Both hosts use
//! the same transport rules; provider key state and retry event ownership stay in
//! the caller until their existing owners are connected to the native client.

use std::time::Duration;

use reqwest::{Client, RequestBuilder, header::HeaderMap};

/// 普通非流式 API 请求的总超时。
pub const STANDARD_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// 非流式**对话补全**的总超时。
///
/// 为什么不能沿用 60 秒：一次 high reasoning + 十万 token 输入的补全，光思考就要两三分钟，
/// 非流式又是「憋完整个回答再一次性返回」——60 秒结构性必然超时，三次重试白烧 195 秒。
/// 实测就踩过：流式跑了 135 秒断包后回落非流式，3×60 秒全超，用户等了三分多钟拿到
/// 一句"模型调用失败"。流式那条路本来就只有 300 秒读空闲、没有总时长上限，回落到更紧的
/// 硬顶是反的。
pub const CHAT_COMPLETION_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
/// 只限制 TCP/TLS 建连阶段，避免 DNS/握手长期卡住。
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// 流式响应的读空闲超时：持续有 SSE chunk 到达时不会触发。
const HTTP_READ_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// 空闲连接在池中最多保留多久后被淘汰。默认(reqwest 90s)偏长；缩短以更快丢弃可能已被
/// 服务端/NAT 静默关闭的连接，降低长时间运行后复用陈旧连接的概率。
const HTTP_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// TCP keepalive：让内核定期探活，及时发现对端已消失的连接。
const HTTP_TCP_KEEPALIVE: Duration = Duration::from_secs(30);
/// HTTP/2 keepalive PING 间隔：h2 单连接多路复用，一条半死连接会拖垮该 host 的全部请求；
/// 定期 PING 让 hyper 主动探测并丢弃死连接（配合 while_idle 覆盖空闲期）。
const HTTP2_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
/// HTTP/2 keepalive PING 超时：PING 无响应超过此值即判定连接死亡。
const HTTP2_KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(15);

/// 为非流式请求设置总超时。流式请求不要用它，否则长回答会被总时长切断。
pub fn with_standard_request_timeout(request: RequestBuilder) -> RequestBuilder {
    request.timeout(STANDARD_HTTP_REQUEST_TIMEOUT)
}

/// 为非流式**对话补全**设置总超时（[`CHAT_COMPLETION_REQUEST_TIMEOUT`]）。
/// 四个适配器（openai / anthropic / gemini / responses）的非流式路径都用它，
/// 别再退回 [`with_standard_request_timeout`]——那是给 embedding / rerank / 文档解析
/// 这类「秒级就该回」的请求用的。
pub fn with_chat_request_timeout(request: RequestBuilder) -> RequestBuilder {
    request.timeout(CHAT_COMPLETION_REQUEST_TIMEOUT)
}

/// 把 JSON body 挂到请求上：`gzip=false` 走普通 `.json()`；`gzip=true` 则序列化后
/// gzip 压缩并设置 `Content-Encoding: gzip`。用于绕开个别供应商前置 WAF 对明文请求体的
/// 误拦（详见 `ModelProvider::compress_request_body`）。压缩任一步失败都安全退回明文。
pub fn attach_json_body(
    request: RequestBuilder,
    body: &serde_json::Value,
    gzip: bool,
) -> RequestBuilder {
    use std::io::Write as _;
    if gzip {
        if let Ok(raw) = serde_json::to_vec(body) {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            if enc.write_all(&raw).is_ok() {
                if let Ok(gz) = enc.finish() {
                    return request
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .header(reqwest::header::CONTENT_ENCODING, "gzip")
                        .body(gz);
                }
            }
        }
    }
    request.json(body)
}

/// 构建 HTTP 客户端：不设置 total timeout，避免活跃 SSE 流在 60 秒处被砍掉。
/// 连接池加保活 + 缩短空闲淘汰：长时间运行后不再复用被对端静默关闭的陈旧连接
/// （见任务 07-24-http-connection-pool-diagnosis）。
pub fn build_http_client() -> Client {
    build_client(false)
}

/// 直连客户端：同样的连接池调优，但忽略系统/环境代理。供关掉「跟随系统代理」的供应商使用。
pub fn build_direct_http_client() -> Client {
    build_client(true)
}

fn build_client(no_proxy: bool) -> Client {
    let builder = Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .read_timeout(HTTP_READ_IDLE_TIMEOUT)
        .pool_idle_timeout(HTTP_POOL_IDLE_TIMEOUT)
        .tcp_keepalive(HTTP_TCP_KEEPALIVE)
        .http2_keep_alive_interval(HTTP2_KEEPALIVE_INTERVAL)
        .http2_keep_alive_timeout(HTTP2_KEEPALIVE_TIMEOUT)
        .http2_keep_alive_while_idle(true);
    let builder = if no_proxy {
        builder.no_proxy()
    } else {
        builder
    };
    builder.build().unwrap_or_else(|err| {
        eprintln!("Failed to build HTTP client: {err}");
        Client::new()
    })
}

/// 流式 UTF-8 增量解码器：SSE 分片可能在多字节字符（如中文 3 字节）中间切开，
/// 逐片 `from_utf8_lossy` 会把半个字符变成替换符。此解码器把不完整的尾字节留到下一片。
#[derive(Default)]
pub struct Utf8StreamDecoder {
    tail: Vec<u8>,
}

impl Utf8StreamDecoder {
    /// 喂入一片原始字节，返回可安全解码的前缀；未构成完整字符的尾字节暂存到下次。
    pub fn push(&mut self, chunk: &[u8]) -> String {
        self.tail.extend_from_slice(chunk);
        let valid = match std::str::from_utf8(&self.tail) {
            Ok(s) => s.len(),
            Err(err) => err.valid_up_to(),
        };
        let out = String::from_utf8_lossy(&self.tail[..valid]).into_owned();
        self.tail.drain(..valid);
        // 单个 UTF-8 字符最多 4 字节：残留超过 4 字节说明是真·非法序列而非跨片切割，
        // 按 lossy 冲掉以免永久卡住。
        if self.tail.len() > 4 {
            let flushed = String::from_utf8_lossy(&self.tail).into_owned();
            self.tail.clear();
            return out + &flushed;
        }
        out
    }
}

/// Claude Code-style bounded exponential backoff, starting at one second.
const RETRY_BASE_DELAY_MS: u64 = 1_000;
const RETRY_MAX_DELAY_MS: u64 = 30_000;
/// 从响应头中解析 Retry-After 值（秒），转换为毫秒延迟
pub(super) fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
}

/// 判断请求错误是否可重试
/// 包括建立连接失败、超时，以及发送请求时连接被代理/上游切断。
pub(crate) fn is_invalid_image_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    ["image", "图片", "base64", "mime", "media type"]
        .iter()
        .any(|word| lower.contains(word))
        && [
            "invalid",
            "decode",
            "corrupt",
            "format",
            "unsupported",
            "too large",
            "size",
            "dimension",
            "exceeds",
            "损坏",
            "格式",
            "超过",
            "超出",
        ]
        .iter()
        .any(|word| lower.contains(word))
}

/// Provider errors that require changing input, credentials or account state.
/// Shared by HTTP rate-limit handling and errors delivered inside a successful SSE response.
pub(crate) fn is_permanent_provider_error(message: &str) -> bool {
    if is_invalid_image_error(message) {
        return true;
    }
    let lower = message.to_ascii_lowercase();
    [
        "insufficient_quota",
        "out of budget",
        "quota exceeded",
        "billing",
        "available balance",
        "monthly usage limit reached",
        "gousagelimiterror",
        "freeusagelimiterror",
        "subscription_sharing_usage_limit_exceeded",
        "subscription_sharing_workplace_individual_usage_limit_exceeded",
        "invalid_request_error",
        "invalid_argument",
        "authentication_error",
        "permission_denied",
        "permission_error",
        "invalid_api_key",
        "unsupported_format",
    ]
    .iter()
    .any(|code| lower.contains(code))
}

pub(super) fn is_retryable_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request() || error.is_body()
}

/// 把 reqwest 错误展开成完整 source 链文本。reqwest 的 `Display` 常年只有一句
/// `error sending request`，真正的原因（DNS 解析 / TCP connect / TLS handshake /
/// connection closed / h2 reset）都在 `std::error::Error::source()` 链里。诊断长时间
/// 运行后的 `statusCode=null` 网络故障必须看到这条链（见任务 07-24-http-connection-pool-diagnosis）。
pub fn format_reqwest_error(error: &reqwest::Error) -> String {
    use std::error::Error as _;
    let mut parts = vec![error.to_string()];
    let mut src = error.source();
    while let Some(err) = src {
        let text = err.to_string();
        // 跳过与上一层完全重复的文案，避免链条里堆叠同一句。
        if parts.last().map(|p| p != &text).unwrap_or(true) {
            parts.push(text);
        }
        src = err.source();
    }
    // 附带 reqwest 的错误类别标记，便于区分是 connect / timeout / body / decode。
    let mut kind = Vec::new();
    if error.is_connect() {
        kind.push("connect");
    }
    if error.is_timeout() {
        kind.push("timeout");
    }
    if error.is_request() {
        kind.push("request");
    }
    if error.is_body() {
        kind.push("body");
    }
    if error.is_decode() {
        kind.push("decode");
    }
    let chain = parts.join(" → ");
    if kind.is_empty() {
        chain
    } else {
        format!("{chain} [{}]", kind.join(","))
    }
}

/// 计算重试延迟
/// 指数退避与 Retry-After 取较大值，避免服务端的小值导致快速耗尽重试。
/// Retry-After 同样受 `RETRY_MAX_DELAY_MS` 封顶：上游（或中转）偶尔返回
/// `Retry-After: 86400` 之类的巨值，照睡会把整次请求挂死一天（只能靠取消轮询打断）。
pub(super) fn retry_delay_ms(attempt: usize, retry_after: Option<u64>) -> u64 {
    let backoff =
        RETRY_BASE_DELAY_MS.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1) as u32));
    backoff
        .max(retry_after.unwrap_or(0).saturating_mul(1000))
        .min(RETRY_MAX_DELAY_MS)
}

fn parse_leading_status_code(value: &str) -> Option<u16> {
    let end = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    if end == 0 {
        return None;
    }
    value[..end].parse().ok()
}

/// 从 HTTP 错误信息中提取状态码
/// 格式约定：`"{label} Error: {status} - {body}"`，
/// status 形如 `"429 Too Many Requests"`，第一段数字即可
/// 兼容少数防御性分支使用的 `"{label} HTTP {status}: {body}"`。
/// 网络错误（reqwest::Error 路径）格式为 `"{label} Error: <reqwest msg>"`，无前导数字 → 返回 None
pub fn extract_status_code(err_msg: &str) -> Option<u16> {
    if let Some(idx) = err_msg.find(" Error: ") {
        let rest = &err_msg[idx + " Error: ".len()..];
        if let Some(code) = parse_leading_status_code(rest) {
            return Some(code);
        }
    }

    if let Some(idx) = err_msg.find(" HTTP ") {
        let rest = &err_msg[idx + " HTTP ".len()..];
        return parse_leading_status_code(rest);
    }

    None
}

/// 判断错误信息是否触发 key failover（外层换 key 决策）
/// 严格按 HTTP 状态码：401/402/403/429 才换 key —— 与 key 直接相关的错误：
/// - 401 鉴权失败（key 被吊销 / 错误）→ 内层不重试，立即换 key
/// - 402 需要付费（账户欠费）→ 立即换 key
/// - 403 权限不足 / 被封禁 → 立即换 key
/// - 429 限流（key 维度配额耗尽）→ 内层先退避重试；达到阈值且有备用 key 时，
///   429 错误冒泡到外层触发换 key（共用同一个重试预算）
/// 其它 4xx（如 400 malformed body）属于请求本身问题，换 key 也无济于事 → 不触发
/// 5xx 由内层退避重试，正常不会到这里（除非耗尽次数；耗尽后非 key 问题，不换 key）
/// 网络错误（timeout / connect 失败）非 key 问题，extract_status_code 返回 None → 不触发
pub fn is_failover_error(err_msg: &str) -> bool {
    matches!(extract_status_code(err_msg), Some(401 | 402 | 403 | 429))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_is_capped_at_max_delay() {
        // 上游/中转返回的巨值 Retry-After 不得被照单全收：否则单次重试会挂死数小时，
        // 只能靠取消轮询打断（B2）。
        assert_eq!(retry_delay_ms(1, Some(86_400)), RETRY_MAX_DELAY_MS);
        assert_eq!(retry_delay_ms(1, Some(u64::MAX)), RETRY_MAX_DELAY_MS);
        // 合理值仍原样生效（未被 clamp 抹平）。
        let small = 2u64;
        assert!(small * 1000 < RETRY_MAX_DELAY_MS, "测试前提：2s 应小于上限");
        assert_eq!(retry_delay_ms(1, Some(small)), small * 1000);
        // 无 Retry-After → 指数退避，同样受上限约束。
        assert!(retry_delay_ms(99, None) <= RETRY_MAX_DELAY_MS);
    }

    #[test]
    fn utf8_decoder_reassembles_split_multibyte() {
        // "温周" 各 3 字节；在字符中间切开分多片喂入，不应产生替换符。
        let bytes = "温周".as_bytes().to_vec();
        let mut dec = Utf8StreamDecoder::default();
        let mut out = String::new();
        // 逐字节喂：最坏情况的边界切割。
        for b in &bytes {
            out.push_str(&dec.push(&[*b]));
        }
        assert_eq!(out, "温周");
        assert!(!out.contains('\u{FFFD}'));
    }

    #[test]
    fn utf8_decoder_passes_ascii_and_complete_chunks() {
        let mut dec = Utf8StreamDecoder::default();
        assert_eq!(dec.push(b"hello "), "hello ");
        assert_eq!(dec.push("世界".as_bytes()), "世界");
    }

    // ===== attach_json_body (gzip) =====

    #[test]
    fn attach_json_body_plain_when_gzip_off() {
        let client = Client::new();
        let body =
            serde_json::json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
        let req = attach_json_body(client.post("http://x.invalid/v1"), &body, false)
            .build()
            .expect("build");
        assert!(
            req.headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .is_none()
        );
        let sent = req
            .body()
            .and_then(|b| b.as_bytes())
            .expect("in-memory body");
        let parsed: serde_json::Value = serde_json::from_slice(sent).expect("plain json");
        assert_eq!(parsed, body);
    }

    #[test]
    fn attach_json_body_gzips_and_round_trips_when_gzip_on() {
        use std::io::Read as _;
        let client = Client::new();
        let body = serde_json::json!({"model": "m", "cmd": "rm -rf /tmp/x && cat /etc/passwd"});
        let req = attach_json_body(client.post("http://x.invalid/v1"), &body, true)
            .build()
            .expect("build");
        assert_eq!(
            req.headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .unwrap(),
            "gzip"
        );
        let gz = req
            .body()
            .and_then(|b| b.as_bytes())
            .expect("in-memory body");
        // 压缩体必须能解回原始 JSON。
        let mut dec = flate2::read::GzDecoder::new(gz);
        let mut raw = Vec::new();
        dec.read_to_end(&mut raw).expect("gunzip");
        let parsed: serde_json::Value = serde_json::from_slice(&raw).expect("round-trip json");
        assert_eq!(parsed, body);
    }

    #[test]
    fn extract_status_code_handles_each_failover_status() {
        assert_eq!(
            extract_status_code("X Error: 401 Unauthorized - body"),
            Some(401)
        );
        assert_eq!(
            extract_status_code("X Error: 402 Payment Required - body"),
            Some(402)
        );
        assert_eq!(
            extract_status_code("X Error: 403 Forbidden - body"),
            Some(403)
        );
        assert_eq!(
            extract_status_code("X Error: 429 Too Many Requests - body"),
            Some(429)
        );
    }

    #[test]
    fn extract_status_code_handles_defensive_http_format() {
        assert_eq!(
            extract_status_code("Stream HTTP 429: rate limited"),
            Some(429)
        );
        assert_eq!(
            extract_status_code("Stream HTTP 401: unauthorized"),
            Some(401)
        );
        assert_eq!(
            extract_status_code("Vision API HTTP 403: forbidden"),
            Some(403)
        );
    }

    #[test]
    fn extract_status_code_handles_non_failover_status() {
        assert_eq!(
            extract_status_code("X Error: 400 Bad Request - body"),
            Some(400)
        );
        assert_eq!(
            extract_status_code("X Error: 500 Internal Server Error - body"),
            Some(500)
        );
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
        assert!(is_failover_error("Stream HTTP 429: rate limited"));
        assert!(is_failover_error("Stream HTTP 401: unauthorized"));
    }

    #[test]
    fn is_failover_error_does_not_trigger_on_400_or_5xx() {
        // 400 是请求 body 问题，不应换 key
        assert!(!is_failover_error("X Error: 400 Bad Request - body"));
        assert!(!is_failover_error("Stream HTTP 400: bad request"));
        // 500 由 send_with_retry 内部退避重试，不应到 failover 层
        assert!(!is_failover_error(
            "X Error: 500 Internal Server Error - body"
        ));
        assert!(!is_failover_error(
            "X Error: 503 Service Unavailable - body"
        ));
    }

    #[test]
    fn is_failover_error_does_not_trigger_on_network_failure() {
        // 网络问题不是 key 的锅
        assert!(!is_failover_error(
            "Stream Error: error sending request: timed out"
        ));
        assert!(!is_failover_error("X Error: connection closed"));
    }

    #[test]
    fn is_failover_error_does_not_trigger_on_body_keywords_alone() {
        // 旧版宽泛匹配 body 含 "billing" / "quota" 会误触发；现版严格按状态码
        assert!(!is_failover_error(
            "X Error: 400 - {\"message\":\"billing issue\"}"
        ));
        assert!(!is_failover_error(
            "X Error: 500 - {\"message\":\"quota exceeded\"}"
        ));
    }

    // ===== retry_delay_ms / parse_retry_after =====

    #[test]
    fn retry_delay_starts_at_one_second_and_caps() {
        // Start at 1s; exponential backoff caps at 30s.
        assert_eq!(retry_delay_ms(1, None), RETRY_BASE_DELAY_MS);
        assert_eq!(retry_delay_ms(2, None), RETRY_BASE_DELAY_MS * 2);
        // The sixth retry would wait 32s, capped at 30s.
        assert_eq!(retry_delay_ms(6, None), RETRY_MAX_DELAY_MS);
        assert_eq!(retry_delay_ms(10, None), RETRY_MAX_DELAY_MS);
    }

    #[test]
    fn retry_delay_uses_retry_after_as_a_minimum() {
        // Small Retry-After values cannot burn the budget faster than backoff.
        assert_eq!(retry_delay_ms(1, Some(7)), 7_000);
        assert_eq!(retry_delay_ms(5, Some(2)), 16_000);
    }

    #[test]
    fn parse_retry_after_reads_seconds_header() {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "12".parse().unwrap());
        assert_eq!(parse_retry_after(&headers), Some(12));

        let empty = HeaderMap::new();
        assert_eq!(parse_retry_after(&empty), None);
    }
}
