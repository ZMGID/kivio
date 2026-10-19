use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn server(
    status: &str,
    body: serde_json::Value,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let body = body.to_string();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
    );
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let count = stream.read(&mut buffer).await.unwrap();
            assert!(count > 0, "request closed before body completed");
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        stream.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8(bytes).unwrap()
    });
    (format!("http://{address}"), task)
}

#[tokio::test]
async fn deepseek_search_uses_raw_api_and_preserves_sources() {
    for suffix in ["", "/v1/", "/v1/responses"] {
        let (base, request) = server("200 OK", serde_json::json!({"results": [
            {"title":"Official guide", "url":"https://example.com/guide", "summary":"Query summary", "snippet":"Short snippet", "content":"", "date":"2026-10-18T00:00:00+00:00", "rerank_score":2.4},
            {"title":"Article", "url":"https://example.com/article", "summary":"", "snippet":"Snippet fallback", "content":"Full Markdown", "date":null, "rerank_score":1.8}
        ]})).await;
        let state = crate::state::test_app_state();
        // Old saved model/prompt fields must not prevent the new API from using
        // the existing provider, credential and gateway address.
        let config: LensWebSearchConfig = serde_json::from_value(serde_json::json!({
            "provider": "deepseek",
            "deepseekApiKey": " test-key ",
            "deepseekBaseUrl": format!("{base}{suffix}"),
            "deepseekModel": "deepseek-v4-flash",
            "deepseekSystemPrompt": "Legacy search instructions",
            "maxResults": 2
        }))
        .unwrap();
        let results = search_web(&state, &config, " Rust site:example.com ", 1, None)
            .await
            .unwrap();
        let request = request.await.unwrap();
        assert!(
            request.starts_with("POST /v1/tools/search HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(request
            .to_lowercase()
            .contains("authorization: bearer test-key\r\n"));
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"query":"Rust site:example.com", "count":2, "need_content":true})
        );
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Official guide");
        assert_eq!(results[0].url, "https://example.com/guide");
        assert_eq!(results[0].content, "Query summary");
        assert_eq!(
            results[0].published_date.as_deref(),
            Some("2026-10-18T00:00:00+00:00")
        );
        assert_eq!(results[0].score, Some(2.4));
        assert_eq!(results[1].content, "Full Markdown");
    }
}

#[tokio::test]
async fn deepseek_empty_search_is_success_and_errors_are_reported() {
    for (status, body, expected) in [
        ("200 OK", serde_json::json!({"results":[]}), None),
        (
            "402 Payment Required",
            serde_json::json!({"error":{"message":"Insufficient Balance"}}),
            Some("Insufficient Balance"),
        ),
        (
            "200 OK",
            serde_json::json!({"unexpected":[]}),
            Some("results"),
        ),
    ] {
        let (base, request) = server(status, body).await;
        let config = LensWebSearchConfig {
            provider: WebSearchProvider::Deepseek,
            deepseek_api_key: "test-key".into(),
            deepseek_base_url: base,
            ..Default::default()
        };
        let result = search_web(&crate::state::test_app_state(), &config, "Rust", 1, None).await;
        request.await.unwrap();
        match expected {
            None => assert!(result.unwrap().is_empty()),
            Some(message) => assert!(result.unwrap_err().contains(message)),
        }
    }
}

#[tokio::test]
async fn deepseek_fetch_routes_explicit_and_followed_provider() {
    for provider in [WebSearchProvider::Deepseek, WebSearchProvider::Brave] {
        let (base, request) = server(
            "200 OK",
            serde_json::json!({"title":"Guide", "content":"# Guide\n\nFull page"}),
        )
        .await;
        let config = LensWebSearchConfig {
            provider,
            fetch_provider: (provider == WebSearchProvider::Brave)
                .then_some(WebSearchProvider::Deepseek),
            deepseek_api_key: "test-key".into(),
            deepseek_base_url: format!("{base}/v1"),
            ..Default::default()
        };
        let page = fetch_web(
            &crate::state::test_app_state(),
            &config,
            "https://example.com/guide",
            1,
            None,
        )
        .await
        .unwrap();
        let request = request.await.unwrap();
        assert!(request.starts_with("POST /v1/tools/fetch HTTP/1.1\r\n"));
        assert!(request
            .to_lowercase()
            .contains("authorization: bearer test-key\r\n"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(request.split_once("\r\n\r\n").unwrap().1)
                .unwrap(),
            serde_json::json!({"url":"https://example.com/guide"})
        );
        assert_eq!(page.title.as_deref(), Some("Guide"));
        assert_eq!(page.text, "# Guide\n\nFull page");
        assert_eq!(page.method, "DeepSeek");
    }
}
