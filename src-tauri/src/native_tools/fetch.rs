use reqwest::{header, Client, Url};
use scraper::{Html, Selector};
use serde_json::Value;

const MAX_FETCH_BYTES: usize = 4 * 1024 * 1024;
const MAX_OUTPUT_CHARS: usize = 120_000;
const MIN_USEFUL_TEXT_CHARS: usize = 600;
const FETCH_TIMEOUT_SECS: u64 = 30;
const JINA_READER_BASE: &str = "https://r.jina.ai/";

#[derive(Debug)]
struct FetchArgs {
    url: String,
    reader_fallback: bool,
}

#[derive(Debug)]
struct FetchOutput {
    url: String,
    final_url: String,
    method: &'static str,
    title: Option<String>,
    content_type: String,
    text: String,
    diagnostics: Vec<String>,
}

#[derive(Debug)]
struct DirectFetch {
    final_url: String,
    content_type: String,
    title: Option<String>,
    text: String,
}

pub async fn web_fetch(_http: &Client, arguments: &Value) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let mut diagnostics = Vec::new();
    // Fetch owns its redirect policy; changing the shared provider client
    // would also change intentional localhost / HTTP provider connections.
    let http = fetch_client()?;
    let mut reader_safe = args.reader_fallback;
    let direct = tokio::time::timeout(
        std::time::Duration::from_secs(FETCH_TIMEOUT_SECS),
        fetch_direct_guarded(http, &args.url, &mut reader_safe),
    )
    .await
    .unwrap_or_else(|_| Err("Fetch timed out".into()));

    match direct {
        Ok(direct) if is_useful_text(&direct.text) => Ok(format_fetch_output(FetchOutput {
            url: args.url,
            final_url: direct.final_url,
            method: "direct",
            title: direct.title,
            content_type: direct.content_type,
            text: direct.text,
            diagnostics,
        })),
        Ok(direct) => {
            diagnostics.push(format!(
                "direct fetch produced only {} readable characters",
                direct.text.chars().count()
            ));
            if !reader_safe {
                return Ok(format_fetch_output(FetchOutput {
                    url: args.url,
                    final_url: direct.final_url,
                    method: "direct-short",
                    title: direct.title,
                    content_type: direct.content_type,
                    text: direct.text,
                    diagnostics,
                }));
            }
            match fetch_jina_reader(http, &args.url).await {
                Ok(reader) if is_useful_text(&reader.text) => {
                    diagnostics.push("reader fallback succeeded".to_string());
                    Ok(format_fetch_output(FetchOutput {
                        url: args.url,
                        final_url: reader.final_url,
                        method: "reader-fallback",
                        title: reader.title,
                        content_type: reader.content_type,
                        text: reader.text,
                        diagnostics,
                    }))
                }
                Ok(reader) => {
                    diagnostics.push(format!(
                        "reader fallback produced only {} readable characters",
                        reader.text.chars().count()
                    ));
                    Ok(format_fetch_output(FetchOutput {
                        url: args.url,
                        final_url: direct.final_url,
                        method: "direct-short",
                        title: direct.title,
                        content_type: direct.content_type,
                        text: direct.text,
                        diagnostics,
                    }))
                }
                Err(err) => {
                    diagnostics.push(format!("reader fallback failed: {err}"));
                    Ok(format_fetch_output(FetchOutput {
                        url: args.url,
                        final_url: direct.final_url,
                        method: "direct-short",
                        title: direct.title,
                        content_type: direct.content_type,
                        text: direct.text,
                        diagnostics,
                    }))
                }
            }
        }
        Err(err) => {
            diagnostics.push(format!("direct fetch failed: {err}"));
            if !reader_safe {
                return Err(diagnostics.join("\n"));
            }
            let reader = fetch_jina_reader(http, &args.url)
                .await
                .map_err(|reader_err| {
                    diagnostics.push(format!("reader fallback failed: {reader_err}"));
                    diagnostics.join("\n")
                })?;
            diagnostics.push("reader fallback succeeded".to_string());
            Ok(format_fetch_output(FetchOutput {
                url: args.url,
                final_url: reader.final_url,
                method: "reader-fallback",
                title: reader.title,
                content_type: reader.content_type,
                text: reader.text,
                diagnostics,
            }))
        }
    }
}

fn parse_args(arguments: &Value) -> Result<FetchArgs, String> {
    let url = arguments
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "web_fetch requires url".to_string())?;

    validate_https_url(url)?;

    Ok(FetchArgs {
        url: url.to_string(),
        reader_fallback: arguments
            .get("reader_fallback")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

fn validate_https_url(url: &str) -> Result<(), String> {
    let parsed = Url::parse(url).map_err(|err| format!("Invalid URL: {err}"))?;
    if parsed.scheme() != "https" {
        return Err("web_fetch only allows https:// URLs".to_string());
    }
    if parsed.host_str().is_none() {
        return Err("web_fetch requires a URL host".to_string());
    }
    Ok(())
}

#[cfg(test)]
async fn fetch_direct(http: &Client, url: &str) -> Result<DirectFetch, String> {
    fetch_direct_guarded(http, url, &mut false).await
}

fn fetch_client() -> Result<&'static Client, String> {
    static CLIENT: std::sync::OnceLock<Result<Client, String>> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
                .build()
                .map_err(|err| format!("Build fetch client failed: {err}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn public_reader_address(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(ip) => {
            let octets = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_documentation()
                && octets[0] < 224
                && !(octets[0] == 100 && (64..=127).contains(&octets[1]))
                && !(octets[0] == 198 && (18..=19).contains(&octets[1]))
                && !(octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        }
        std::net::IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return public_reader_address(ip.into());
            }
            let segments = ip.segments();
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && segments[0] & 0xfe00 != 0xfc00
                && segments[0] & 0xffc0 != 0xfe80
                && segments[0] & 0xffc0 != 0xfec0
                && !(segments[0] == 0x2001 && segments[1] == 0xdb8)
        }
    }
}

async fn reader_url_is_public(url: &Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_matches(['[', ']']);
    if let Ok(address) = host.parse() {
        return public_reader_address(address);
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if !host.contains('.')
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }
    // Internal URLs remain fetchable. This classification only prevents
    // sending their URL to an external reader; it is not an SSRF firewall.
    let public = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::lookup_host((host.as_str(), url.port_or_known_default().unwrap_or(443))),
    )
    .await
    {
        Ok(Ok(addresses)) => {
            let addresses: Vec<_> = addresses.collect();
            !addresses.is_empty()
                && addresses
                    .iter()
                    .all(|address| public_reader_address(address.ip()))
        }
        _ => false,
    };
    public
}

async fn fetch_direct_guarded(
    http: &Client,
    url: &str,
    reader_safe: &mut bool,
) -> Result<DirectFetch, String> {
    let mut url = Url::parse(url).map_err(|err| format!("Invalid URL: {err}"))?;
    let mut redirect_count = 0;
    let response = loop {
        if *reader_safe {
            // Cancellation during DNS classification must disable fallback too.
            *reader_safe = false;
            *reader_safe = reader_url_is_public(&url).await;
        }
        let response = http
            .get(url.clone())
            .headers(browser_like_headers())
            .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|err| format!("Fetch failed: {err}"))?;
        if !response.status().is_redirection() {
            break response;
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .ok_or_else(|| "Redirect missing Location".to_string())?
            .to_str()
            .map_err(|err| format!("Invalid redirect: {err}"))?;
        let next = url
            .join(location)
            .map_err(|err| format!("Invalid redirect URL: {err}"))?;
        validate_https_url(next.as_str())?;
        redirect_count += 1;
        if redirect_count > 10 {
            return Err("Too many fetch redirects".into());
        }
        url = next;
    };

    let status = response.status();
    let final_url = response.url().to_string();
    if !status.is_success() {
        let error_body = read_error_body(response).await;
        return Err(format_http_error("HTTP", status, error_body.as_deref()));
    }

    let content_type = response_content_type(&response);
    let (bytes, truncated) = read_limited_body(response, MAX_FETCH_BYTES).await?;
    if truncated {
        return Err(format!("Response too large (max {MAX_FETCH_BYTES} bytes)"));
    }

    let body = String::from_utf8_lossy(&bytes).into_owned();
    let (title, text) = if content_type.contains("text/html") || looks_like_html(&body) {
        html_to_readable_text(&body)
    } else {
        (None, collapse_whitespace(&body))
    };

    Ok(DirectFetch {
        final_url,
        content_type,
        title,
        text,
    })
}

async fn fetch_jina_reader(http: &Client, url: &str) -> Result<DirectFetch, String> {
    let reader_url = jina_reader_url(url)?;
    let response = http
        .get(&reader_url)
        .headers(browser_like_headers())
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|err| format!("Reader fetch failed: {err}"))?;

    let status = response.status();
    let final_url = response.url().to_string();
    if !status.is_success() {
        let error_body = read_error_body(response).await;
        return Err(format_http_error(
            "Reader HTTP",
            status,
            error_body.as_deref(),
        ));
    }

    let content_type = response_content_type(&response);
    let (bytes, truncated) = read_limited_body(response, MAX_FETCH_BYTES).await?;
    if truncated {
        return Err(format!(
            "Reader response too large (max {MAX_FETCH_BYTES} bytes)"
        ));
    }

    let body = String::from_utf8_lossy(&bytes).into_owned();
    let title = markdown_title(&body);
    let text = collapse_markdown_whitespace(&body);

    Ok(DirectFetch {
        final_url,
        content_type,
        title,
        text,
    })
}

fn browser_like_headers() -> header::HeaderMap {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_static(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36 Kivio/1.0",
        ),
    );
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,text/plain;q=0.8,*/*;q=0.5",
        ),
    );
    headers.insert(
        header::ACCEPT_LANGUAGE,
        header::HeaderValue::from_static("zh-CN,zh;q=0.9,en-US;q=0.8,en;q=0.7"),
    );
    headers
}

fn response_content_type(response: &reqwest::Response) -> String {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase()
}

async fn read_limited_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<(Vec<u8>, bool), String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        // Success responses reject the declared size without waiting for EOF.
        if limit == MAX_FETCH_BYTES {
            return Ok((Vec::new(), true));
        }
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| format!("Read body failed: {err}"))?
    {
        let remaining = limit.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() > remaining || (limit != MAX_FETCH_BYTES && bytes.len() == limit) {
            return Ok((bytes, true));
        }
    }
    Ok((bytes, false))
}

async fn read_error_body(response: reqwest::Response) -> Option<String> {
    let (bytes, _) = read_limited_body(response, 4096).await.ok()?;
    if bytes.is_empty() {
        return None;
    }
    Some(truncate_chars(
        &collapse_whitespace(&String::from_utf8_lossy(&bytes)),
        800,
    ))
}

fn format_http_error(prefix: &str, status: reqwest::StatusCode, body: Option<&str>) -> String {
    match body.filter(|body| !body.trim().is_empty()) {
        Some(body) => format!("{prefix} {status}: {body}"),
        None => format!("{prefix} {status}"),
    }
}

fn jina_reader_url(url: &str) -> Result<String, String> {
    validate_https_url(url)?;
    Ok(format!("{JINA_READER_BASE}{url}"))
}

fn looks_like_html(body: &str) -> bool {
    let lower = body
        .chars()
        .take(512)
        .collect::<String>()
        .to_ascii_lowercase();
    lower.contains("<!doctype html") || lower.contains("<html") || lower.contains("<body")
}

fn html_to_readable_text(html: &str) -> (Option<String>, String) {
    let document = Html::parse_document(html);
    let title = first_meta_content(&document, "meta[property=\"og:title\"]")
        .or_else(|| first_selector_text(&document, "title"))
        .or_else(|| first_selector_text(&document, "h1"));

    for selector in [
        "article",
        "main",
        "[role=\"main\"]",
        ".post-content",
        ".entry-content",
        ".article-content",
        ".markdown-body",
    ] {
        if let Some(text) = largest_selector_text(&document, selector) {
            if text.chars().count() >= 80 {
                return (title.map(|value| collapse_whitespace(&value)), text);
            }
        }
    }

    let text =
        largest_selector_text(&document, "body").unwrap_or_else(|| fallback_html_to_text(html));

    (title.map(|value| collapse_whitespace(&value)), text)
}

/// Extract readable plain text from an HTML document for knowledge-base ingest.
/// Reuses the same article-extraction the `web_fetch` tool uses; prepends the
/// page title as an `# h1` so it survives chunking.
pub(crate) fn html_to_text(html: &str) -> String {
    let (title, body) = html_to_readable_text(html);
    match title {
        Some(t) if !t.trim().is_empty() => format!("# {t}\n\n{body}"),
        _ => body,
    }
}

fn first_selector_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document.select(&selector).find_map(|element| {
        let text = element.text().collect::<Vec<_>>().join(" ");
        let collapsed = collapse_whitespace(&text);
        if collapsed.is_empty() {
            None
        } else {
            Some(collapsed)
        }
    })
}

fn first_meta_content(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document.select(&selector).find_map(|element| {
        let collapsed = collapse_whitespace(element.value().attr("content").unwrap_or(""));
        if collapsed.is_empty() {
            None
        } else {
            Some(collapsed)
        }
    })
}

fn largest_selector_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .filter_map(|element| {
            let text = collapse_whitespace(&element.text().collect::<Vec<_>>().join(" "));
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        })
        .max_by_key(|text| text.chars().count())
}

fn fallback_html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    for tag in ["script", "style", "noscript", "svg", "canvas"] {
        while let Some(start) = s.to_ascii_lowercase().find(&format!("<{tag}")) {
            if let Some(end) = s[start..].to_ascii_lowercase().find(&format!("</{tag}>")) {
                let remove_end = start + end + tag.len() + 3;
                s.replace_range(start..remove_end.min(s.len()), " ");
            } else {
                break;
            }
        }
    }
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    collapse_whitespace(&out)
}

fn markdown_title(markdown: &str) -> Option<String> {
    markdown.lines().find_map(|line| {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("Title:") {
            return non_empty_string(title.trim());
        }
        if let Some(title) = trimmed.strip_prefix("# ") {
            return non_empty_string(title.trim());
        }
        None
    })
}

fn collapse_markdown_whitespace(text: &str) -> String {
    let mut out = String::new();
    let mut blank_count = 0;
    for line in text.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                out.push('\n');
            }
            continue;
        }
        blank_count = 0;
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

fn collapse_whitespace(text: &str) -> String {
    let mut out = String::new();
    let mut prev_space = false;
    for ch in html_escape::decode_html_entities(text).chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

fn non_empty_string(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn is_useful_text(text: &str) -> bool {
    text.chars().count() >= MIN_USEFUL_TEXT_CHARS
}

fn format_fetch_output(output: FetchOutput) -> String {
    let mut out = String::new();
    out.push_str(&format!("URL: {}\n", output.url));
    if output.final_url != output.url {
        out.push_str(&format!("Final URL: {}\n", output.final_url));
    }
    out.push_str(&format!("Fetch method: {}\n", output.method));
    if let Some(title) = output.title.filter(|title| !title.trim().is_empty()) {
        out.push_str(&format!("Title: {}\n", title.trim()));
    }
    if !output.content_type.is_empty() {
        out.push_str(&format!("Content-Type: {}\n", output.content_type));
    }
    if !output.diagnostics.is_empty() {
        out.push_str("Diagnostics:\n");
        for item in output.diagnostics {
            out.push_str("- ");
            out.push_str(&item);
            out.push('\n');
        }
    }
    out.push('\n');
    out.push_str(&truncate_chars(&output.text, MAX_OUTPUT_CHARS));
    out
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("\n\n[Truncated]");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn internal_urls_disable_external_reader_without_blocking_direct_fetch() {
        for url in [
            "https://localhost/private",
            "https://127.0.0.1/private",
            "https://[::1]/private",
            "https://internal.local/private",
            "https://[fec0::1]/",
        ] {
            validate_https_url(url).unwrap();
            assert!(!reader_url_is_public(&Url::parse(url).unwrap()).await);
        }
        assert!(!public_reader_address("::ffff:10.0.0.1".parse().unwrap()));
        assert!(!public_reader_address("100.64.0.1".parse().unwrap()));
    }

    #[tokio::test]
    async fn redirect_to_http_is_rejected_before_second_request() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: http://{address}/insecure\r\nContent-Length: 0\r\n\r\n").as_bytes()).await.unwrap();
            tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                .await
                .is_ok()
        });
        let err = fetch_direct(fetch_client().unwrap(), &format!("http://{address}/"))
            .await
            .unwrap_err();
        assert!(err.contains("only allows https"));
        assert!(
            !server.await.unwrap(),
            "insecure redirect must not be contacted"
        );
    }

    #[tokio::test]
    async fn error_body_is_bounded_before_eof() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 500 Error\r\nContent-Length: 5000000\r\n\r\n")
                .await
                .unwrap();
            socket.write_all(&vec![b'x'; 4096]).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        });
        let response = Client::new()
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap();
        let body = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            read_error_body(response),
        )
        .await
        .expect("error budget must not wait for EOF")
        .unwrap();
        assert!(body.len() < 1000);
        server.abort();
    }

    #[tokio::test]
    async fn response_limit_stops_before_stream_eof() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5000000\r\n\r\n")
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        });
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            fetch_direct(&Client::new(), &format!("http://{address}/")),
        )
        .await;
        server.abort();
        assert!(result
            .expect("size limit must reject before body EOF")
            .unwrap_err()
            .contains("large"));
    }

    #[test]
    fn parse_args_accepts_https_url_and_default_fallback() {
        let args = parse_args(&json!({ "url": "https://example.com/page" })).expect("args");
        assert_eq!(args.url, "https://example.com/page");
        assert!(args.reader_fallback);
    }

    #[test]
    fn parse_args_rejects_non_https_url() {
        let err = parse_args(&json!({ "url": "http://example.com/page" })).unwrap_err();
        assert!(err.contains("https"));
    }

    #[test]
    fn jina_reader_url_wraps_https_url() {
        let reader_url = jina_reader_url("https://example.com/a?b=1").expect("reader url");
        assert_eq!(reader_url, "https://r.jina.ai/https://example.com/a?b=1");
    }

    #[test]
    fn html_to_readable_text_prefers_article_content() {
        let html = r#"
          <html>
            <head><title>Example Article</title></head>
            <body>
              <nav>Home Products Pricing</nav>
              <article>
                <h1>Useful Heading</h1>
                <p>This is the useful article body.</p>
                <p>It should be extracted before navigation text.</p>
              </article>
            </body>
          </html>
        "#;

        let (title, text) = html_to_readable_text(html);
        assert_eq!(title.as_deref(), Some("Example Article"));
        assert!(text.contains("This is the useful article body"));
        assert!(!text.starts_with("Home Products Pricing"));
    }

    #[test]
    fn format_fetch_output_includes_diagnostics() {
        let output = format_fetch_output(FetchOutput {
            url: "https://example.com".to_string(),
            final_url: "https://example.com".to_string(),
            method: "reader-fallback",
            title: Some("Example".to_string()),
            content_type: "text/plain".to_string(),
            text: "hello".to_string(),
            diagnostics: vec!["direct fetch failed: HTTP 403".to_string()],
        });

        assert!(output.contains("Fetch method: reader-fallback"));
        assert!(output.contains("direct fetch failed"));
        assert!(output.contains("hello"));
    }

    #[test]
    fn format_http_error_includes_compact_body() {
        let error = format_http_error(
            "Reader HTTP",
            reqwest::StatusCode::BAD_REQUEST,
            Some("{\"message\":\"Domain could not be resolved\"}"),
        );

        assert!(error.contains("400 Bad Request"));
        assert!(error.contains("Domain could not be resolved"));
    }
}
