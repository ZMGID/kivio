//! 把底层 provider / 网络错误串映射成一句**可读、可操作**的中文提示。
//!
//! agent loop 把失败原样冒泡成形如
//! `"Chat tools planning Error: 402 Payment Required - {\"error\":{...}} (attempt 1/2)"`
//! 的字符串：里面夹着阶段名、HTTP 状态、provider 原始 JSON、以及重试计数 `(attempt N/M)`。
//! 直接展示给用户既吓人又没用。[`friendly_error`] 按 HTTP 状态码 + 关键词识别常见失败类别，
//! 给出一句简短的中文处置建议；无法识别时退化为「请求失败：<裁掉 JSON 的首行>」，用户仍能看到
//! 有用信息而不是一大坨 JSON。

/// 通用兜底信息里保留的原始文本最大字符数（char-aware）。
const GENERIC_RAW_CAP: usize = 160;

/// 把一个底层错误串映射成一句简短、可操作的中文提示。
///
/// 识别优先级：先看夹在串里的 HTTP 状态码（最可靠），再看 provider 文案关键词。
/// 展示前会剥掉结尾的 `(attempt N/M)` 噪声。`"cancelled"` 原样返回（取消有自己的 UI 路径，
/// 不应被当成错误二次包装）。
pub fn friendly_error(raw: &str) -> String {
    let trimmed = strip_attempt_suffix(raw).trim().to_string();
    let lower = trimmed.to_lowercase();

    // 取消：保持现有取消路径，原样返回（调用方自行决定是否走 (cancelled) 标记）。
    if lower == "cancelled" || lower.contains("cancelled") {
        return trimmed;
    }

    // 402 / 余额不足 / 需付费。
    if status_present(&trimmed, 402)
        || lower.contains("insufficient balance")
        || lower.contains("payment required")
    {
        return "供应商余额不足 (402)。请充值、更换 API key，或用 /model 切换到其他模型。"
            .to_string();
    }

    // 401 / 403 鉴权失败。
    if status_present(&trimmed, 401)
        || status_present(&trimmed, 403)
        || lower.contains("unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("forbidden")
    {
        return "鉴权失败 (401/403)。检查该供应商的 API key 是否正确/有权限。".to_string();
    }

    // 429 限流。
    if status_present(&trimmed, 429)
        || lower.contains("rate limit")
        || lower.contains("too many requests")
    {
        return "请求过于频繁 (429)。稍后重试，或换 key / 用 /model 切换模型。".to_string();
    }

    // 5xx 服务端不可用。
    if status_present(&trimmed, 500)
        || status_present(&trimmed, 502)
        || status_present(&trimmed, 503)
        || status_present(&trimmed, 504)
        || lower.contains("server error")
        || lower.contains("bad gateway")
        || lower.contains("service unavailable")
    {
        return "供应商服务暂时不可用 (5xx)。稍后重试或用 /model 切换模型。".to_string();
    }

    // 超时。
    if lower.contains("timed out") || lower.contains("timeout") {
        return "请求超时。检查网络或供应商 base_url，稍后重试。".to_string();
    }

    // 网络连接失败。
    if lower.contains("error sending request")
        || lower.contains("connection")
        || lower.contains("connect")
        || lower.contains("dns")
        || lower.contains("network")
    {
        return "网络连接失败。检查网络或供应商 base_url。".to_string();
    }

    // 兜底：裁掉 JSON 大块，只留首行的有用文本（封顶 GENERIC_RAW_CAP 个字符）。
    format!("请求失败：{}", generic_summary(&trimmed))
}

/// 是否在串里出现了**作为独立 token** 的指定 HTTP 状态码（避免把 "2402" 之类误判成 402）。
/// 识别 `Error: 402 ...` / `402 Payment` / `(402)` / `status 402` 等常见形态。
fn status_present(text: &str, status: u16) -> bool {
    let needle = status.to_string();
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(rel) = text[from..].find(&needle) {
        let start = from + rel;
        let end = start + needle.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_digit();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_digit();
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

/// 剥掉结尾的重试计数噪声 `(attempt N/M)`（loop 给 retry 失败加的尾巴）。
fn strip_attempt_suffix(raw: &str) -> String {
    let trimmed = raw.trim_end();
    if let Some(idx) = trimmed.rfind("(attempt ") {
        if trimmed.ends_with(')') {
            return trimmed[..idx].trim_end().to_string();
        }
    }
    trimmed.to_string()
}

/// 兜底摘要：取首行（遇到 JSON 起始 `{`/`[` 或换行就截断），再封顶到 GENERIC_RAW_CAP 字符。
fn generic_summary(text: &str) -> String {
    // 截到第一处 JSON 起始或换行之前。
    let mut cut = text.len();
    for (idx, ch) in text.char_indices() {
        if ch == '\n' || ch == '{' || ch == '[' {
            cut = idx;
            break;
        }
    }
    let head = text[..cut].trim();
    let head = if head.is_empty() { text.trim() } else { head };
    let capped: String = head.chars().take(GENERIC_RAW_CAP).collect();
    if capped.chars().count() < head.chars().count() {
        format!("{capped}…")
    } else {
        capped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_402: &str = "Chat tools planning Error: 402 Payment Required - {\"error\":{\"message\":\"Insufficient Balance\",\"type\":\"insufficient_quota\"}} (attempt 1/2)";

    #[test]
    fn maps_real_402_to_friendly_without_json_or_attempt() {
        let out = friendly_error(REAL_402);
        assert!(out.contains("余额"), "should mention balance: {out}");
        assert!(out.contains("/model"), "should suggest /model: {out}");
        // The raw JSON blob and the retry-count noise must be gone.
        assert!(!out.contains('{'), "must not leak raw JSON: {out}");
        assert!(!out.contains("attempt"), "must strip (attempt N/M): {out}");
        assert!(
            !out.contains("Insufficient Balance"),
            "must not echo raw msg: {out}"
        );
    }

    #[test]
    fn maps_401_and_403_to_auth() {
        assert!(friendly_error("Chat synthesis Error: 401 Unauthorized - {...}").contains("鉴权"));
        assert!(friendly_error("Error: 403 Forbidden").contains("鉴权"));
        assert!(friendly_error("invalid api key provided").contains("鉴权"));
    }

    #[test]
    fn maps_429_to_rate_limit() {
        assert!(friendly_error("Error: 429 Too Many Requests (attempt 2/3)").contains("频繁"));
        assert!(friendly_error("hit the rate limit").contains("频繁"));
    }

    #[test]
    fn maps_5xx_to_unavailable() {
        let out = friendly_error("Chat planning Error: 503 Service Unavailable");
        assert!(out.contains("5xx") || out.contains("不可用"), "got: {out}");
        assert!(friendly_error("Error: 502 Bad Gateway").contains("不可用"));
    }

    #[test]
    fn maps_timeout() {
        assert!(friendly_error("operation timed out").contains("超时"));
    }

    #[test]
    fn maps_connection_error() {
        assert!(friendly_error("error sending request for url").contains("网络"));
    }

    #[test]
    fn cancelled_is_passed_through() {
        assert_eq!(friendly_error("cancelled"), "cancelled");
    }

    #[test]
    fn unrecognized_error_is_generic_and_trimmed() {
        let raw = "something totally unexpected went wrong here";
        let out = friendly_error(raw);
        assert!(out.starts_with("请求失败"), "got: {out}");
        assert!(
            out.contains("something totally unexpected"),
            "keeps the useful text: {out}"
        );
        assert!(!out.contains('{'));
    }

    #[test]
    fn generic_drops_giant_json_blob() {
        let raw = "weird provider failure - {\"error\":{\"message\":\"a very long blob that should not be shown to the user at all because it is huge and noisy\"}}";
        let out = friendly_error(raw);
        assert!(!out.contains('{'), "must not show the JSON blob: {out}");
        assert!(
            out.contains("weird provider failure"),
            "keeps the head: {out}"
        );
    }

    #[test]
    fn status_present_ignores_substring_digits() {
        // "2402" must not be read as a 402.
        assert!(!status_present("code 2402 weird", 402));
        assert!(status_present("Error: 402 Payment", 402));
        assert!(status_present("(402)", 402));
    }

    #[test]
    fn strip_attempt_suffix_removes_noise() {
        assert_eq!(strip_attempt_suffix("boom (attempt 1/2)"), "boom");
        assert_eq!(strip_attempt_suffix("no suffix here"), "no suffix here");
    }
}
