use serde::Deserialize;

use crate::{
    api::send_with_retry,
    settings::{LensWebSearchConfig, WebSearchProvider},
    state::AppState,
};

#[derive(Debug, Clone)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub content: String,
    pub published_date: Option<String>,
    pub score: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct TavilySearchResponse {
    #[serde(default)]
    answer: Option<String>,
    #[serde(default)]
    results: Vec<TavilySearchResult>,
}

#[derive(Debug, Deserialize)]
struct TavilySearchResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    published_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExaSearchResponse {
    #[serde(default)]
    results: Vec<ExaSearchResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExaSearchResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    highlights: Vec<String>,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    published_date: Option<String>,
}

pub async fn search_web(
    state: &AppState,
    config: &LensWebSearchConfig,
    query: &str,
    retry_attempts: usize,
) -> Result<Vec<WebSearchResult>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    match config.provider {
        WebSearchProvider::Tavily => search_tavily(state, config, query, retry_attempts).await,
        WebSearchProvider::Exa => search_exa(state, config, query, retry_attempts).await,
    }
}

async fn search_tavily(
    state: &AppState,
    config: &LensWebSearchConfig,
    query: &str,
    retry_attempts: usize,
) -> Result<Vec<WebSearchResult>, String> {
    let api_key = config.tavily_api_key.trim();
    if api_key.is_empty() {
        return Err("Tavily API key is not configured".to_string());
    }

    let max_results = config.max_results.clamp(1, 10);
    let search_depth = match config.search_depth.as_str() {
        "ultra-fast" | "fast" | "basic" | "advanced" => config.search_depth.as_str(),
        _ => "basic",
    };
    let body = serde_json::json!({
        "query": query,
        "search_depth": search_depth,
        "max_results": max_results,
        "include_answer": true,
        "include_raw_content": false,
        "include_images": false,
        "include_favicon": false,
    });

    let response = send_with_retry("Tavily search", retry_attempts, || {
        state
            .http
            .post("https://api.tavily.com/search")
            .bearer_auth(api_key)
            .json(&body)
            .send()
    })
    .await?;

    let raw = response
        .text()
        .await
        .map_err(|err| format!("Tavily search read body: {err}"))?;
    let parsed: TavilySearchResponse = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "Tavily search parse JSON: {} (body: {})",
            err,
            raw.chars().take(500).collect::<String>()
        )
    })?;

    let mut results: Vec<WebSearchResult> = parsed
        .results
        .into_iter()
        .filter(|result| !result.url.trim().is_empty())
        .map(|result| WebSearchResult {
            title: result.title.trim().to_string(),
            url: result.url.trim().to_string(),
            content: result.content.trim().to_string(),
            published_date: result.published_date,
            score: result.score,
        })
        .collect();

    if let Some(answer) = parsed.answer.as_deref().filter(|answer| !answer.trim().is_empty()) {
        results.insert(
            0,
            WebSearchResult {
                title: "Tavily answer".to_string(),
                url: "https://api.tavily.com/search".to_string(),
                content: answer.trim().to_string(),
                published_date: None,
                score: None,
            },
        );
    }

    Ok(results)
}

async fn search_exa(
    state: &AppState,
    config: &LensWebSearchConfig,
    query: &str,
    retry_attempts: usize,
) -> Result<Vec<WebSearchResult>, String> {
    let api_key = config.exa_api_key.trim();
    if api_key.is_empty() {
        return Err("Exa API key is not configured".to_string());
    }

    let max_results = config.max_results.clamp(1, 10);
    let body = serde_json::json!({
        "query": query,
        "numResults": max_results,
        "contents": {
            "highlights": true
        }
    });

    let response = send_with_retry("Exa search", retry_attempts, || {
        state
            .http
            .post("https://api.exa.ai/search")
            .header("x-api-key", api_key)
            .json(&body)
            .send()
    })
    .await?;

    let raw = response
        .text()
        .await
        .map_err(|err| format!("Exa search read body: {err}"))?;
    let parsed: ExaSearchResponse = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "Exa search parse JSON: {} (body: {})",
            err,
            raw.chars().take(500).collect::<String>()
        )
    })?;

    Ok(parsed
        .results
        .into_iter()
        .filter(|result| !result.url.trim().is_empty())
        .map(|result| {
            let content = if !result.highlights.is_empty() {
                result.highlights.join("\n")
            } else if !result.summary.trim().is_empty() {
                result.summary
            } else {
                result.text
            };
            WebSearchResult {
                title: result.title.trim().to_string(),
                url: result.url.trim().to_string(),
                content: content.trim().to_string(),
                published_date: result.published_date,
                score: result.score,
            }
        })
        .collect())
}

pub fn format_web_context(results: &[WebSearchResult]) -> String {
    if results.is_empty() {
        return String::new();
    }

    let mut lines = Vec::with_capacity(results.len() * 5 + 4);
    lines.push("Web search context:".to_string());
    lines.push(
        "Use only these sources for current web facts. Cite sources with [1], [2], etc. If the sources are insufficient, say so."
            .to_string(),
    );

    for (idx, result) in results.iter().enumerate() {
        let title = if result.title.is_empty() {
            "Untitled"
        } else {
            result.title.as_str()
        };
        lines.push(format!("[{}] {}", idx + 1, title));
        lines.push(format!("URL: {}", result.url));
        if let Some(date) = result
            .published_date
            .as_deref()
            .filter(|d| !d.trim().is_empty())
        {
            lines.push(format!("Published: {}", date.trim()));
        }
        if let Some(score) = result.score {
            lines.push(format!("Score: {:.3}", score));
        }
        if !result.content.is_empty() {
            let snippet: String = result.content.chars().take(1200).collect();
            lines.push(format!("Snippet: {}", snippet));
        }
    }

    lines.join("\n")
}
