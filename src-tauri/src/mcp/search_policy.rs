use crate::settings::WebSearchProvider;
fn tinyfish_mcp_configured(settings: &crate::settings::Settings) -> bool {
    let url = settings.lens.web_search.tinyfish_mcp_url.trim();
    if url.is_empty() {
        return false;
    }
    if settings
        .lens
        .web_search
        .tinyfish_mcp_auth
        .as_ref()
        .is_some_and(|auth| !auth.access_token.trim().is_empty())
    {
        return true;
    }
    let normalized = url.trim_end_matches('/');
    settings.chat_tools.servers.iter().any(|server| {
        if server.url.trim().trim_end_matches('/') != normalized {
            return false;
        }
        server
            .headers
            .get("Authorization")
            .is_some_and(|value| !value.trim().is_empty())
            || server
                .auth
                .as_ref()
                .is_some_and(|auth| !auth.access_token.trim().is_empty())
    })
}

pub(crate) fn web_search_provider_configured(
    settings: &crate::settings::Settings,
    provider: WebSearchProvider,
) -> bool {
    match provider {
        WebSearchProvider::Tavily => !settings.lens.web_search.tavily_api_key.trim().is_empty(),
        WebSearchProvider::Exa => !settings.lens.web_search.exa_api_key.trim().is_empty(),
        WebSearchProvider::ExaMcp => !settings.lens.web_search.exa_mcp_url.trim().is_empty(),
        WebSearchProvider::Ollama => !settings.lens.web_search.ollama_api_key.trim().is_empty(),
        WebSearchProvider::Grok => !settings.lens.web_search.grok_api_key.trim().is_empty(),
        WebSearchProvider::Deepseek => !settings.lens.web_search.deepseek_api_key.trim().is_empty(),
        WebSearchProvider::Brave => !settings.lens.web_search.brave_api_key.trim().is_empty(),
        WebSearchProvider::Serper => !settings.lens.web_search.serper_api_key.trim().is_empty(),
        WebSearchProvider::Bocha => !settings.lens.web_search.bocha_api_key.trim().is_empty(),
        WebSearchProvider::Zhipu => !settings.lens.web_search.zhipu_api_key.trim().is_empty(),
        WebSearchProvider::Tinyfish => !settings.lens.web_search.tinyfish_api_key.trim().is_empty(),
        WebSearchProvider::TinyfishMcp => tinyfish_mcp_configured(settings),
        WebSearchProvider::Searxng => !settings.lens.web_search.searxng_base_url.trim().is_empty(),
        WebSearchProvider::Kimi => !settings.lens.web_search.kimi_api_key.trim().is_empty(),
        WebSearchProvider::Unknown => false,
    }
}

pub(crate) fn web_search_configured(settings: &crate::settings::Settings) -> bool {
    web_search_provider_configured(settings, settings.lens.web_search.provider)
}

pub(crate) fn web_fetch_configured(settings: &crate::settings::Settings) -> bool {
    crate::web_search::resolved_fetch_provider(&settings.lens.web_search)
        .is_some_and(|provider| web_search_provider_configured(settings, provider))
}
