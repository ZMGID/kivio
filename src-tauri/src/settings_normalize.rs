// Shared settings normalization; included by both native and Tauri owners.
fn sync_legacy_chat_model_fields(settings: &mut Settings) {
    let (provider_id, model) = settings.effective_chat_model();
    settings.chat_provider_id = provider_id;
    settings.chat_model = model;
}

fn mirror_explicit_chat_default_for_persistence(settings: &mut Settings) {
    if settings.default_models.chat.is_configured() {
        settings.chat_provider_id = settings.default_models.chat.provider_id.clone();
        settings.chat_model = settings.default_models.chat.model.clone();
    } else {
        settings.chat_provider_id.clear();
        settings.chat_model.clear();
    }
}

pub fn sanitize_settings(mut settings: Settings) -> Settings {
    settings.reading_target_lang = resolve_reading_language(&settings);
    let token = settings.chat_tools.playwright_extension_token.trim();
    let token = token.strip_prefix("export ").unwrap_or(token).trim();
    let token = token
        .strip_prefix("PLAYWRIGHT_MCP_EXTENSION_TOKEN=")
        .unwrap_or(token)
        .trim();
    let token = token
        .strip_prefix('\"')
        .and_then(|s| s.strip_suffix('\"'))
        .or_else(|| token.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(token);
    settings.chat_tools.playwright_extension_token = token.trim().to_owned();
    // RapidOCR 档位归一:非法值回落到各自默认(截图=standard,文档处理=high)。
    if settings.screenshot_translation.rapid_ocr_tier != "standard"
        && settings.screenshot_translation.rapid_ocr_tier != "high"
    {
        settings.screenshot_translation.rapid_ocr_tier = default_rapid_ocr_tier();
    }
    if settings.document_processing.rapid_ocr_tier != "standard"
        && settings.document_processing.rapid_ocr_tier != "high"
    {
        settings.document_processing.rapid_ocr_tier = default_rapid_ocr_tier_high();
    }
    // 1. 从旧版配置迁移
    if settings.providers.is_empty() {
        // 迁移翻译提供商
        if let Some(old_openai) = settings.openai.take() {
            let legacy_key = old_openai.api_key.trim().to_string();
            let api_keys = if legacy_key.is_empty() {
                vec![]
            } else {
                vec![legacy_key]
            };
            settings.providers.push(ModelProvider {
                id: "default-translator".to_string(),
                name: "OpenAI (Translator)".to_string(),
                api_keys,
                api_key_legacy: None,
                base_url: old_openai.base_url,
                available_models: vec![],
                enabled_models: vec![old_openai.model.clone()],
                enabled: true,
                api_format: "openai".to_string(),
                model_overrides: std::collections::HashMap::new(),
                compress_request_body: false,
                request: Default::default(),
                active_key_index: 0,
            });
            settings.translator_provider_id = "default-translator".to_string();
            settings.translator_model = old_openai.model;
        }
        // 迁移 OCR 提供商
        if let Some(old_ocr) = settings.screenshot_translation.openai.take() {
            let legacy_key = old_ocr.api_key.trim().to_string();
            let api_keys = if legacy_key.is_empty() {
                vec![]
            } else {
                vec![legacy_key]
            };
            settings.providers.push(ModelProvider {
                id: "default-ocr".to_string(),
                name: "OpenAI (OCR)".to_string(),
                api_keys,
                api_key_legacy: None,
                base_url: old_ocr.base_url,
                available_models: vec![],
                enabled_models: vec![old_ocr.model.clone()],
                enabled: true,
                api_format: "openai".to_string(),
                model_overrides: std::collections::HashMap::new(),
                compress_request_body: false,
                request: Default::default(),
                active_key_index: 0,
            });
            settings.screenshot_translation.provider_id = "default-ocr".to_string();
            settings.screenshot_translation.model = old_ocr.model;
        }
    }

    for provider in &mut settings.providers {
        provider::sanitize_provider(provider);
    }

    let removed_legacy_local_provider_ids: std::collections::HashSet<String> = settings
        .providers
        .iter()
        .filter(|provider| provider.base_url == LEGACY_APPLE_INTELLIGENCE_BASE_URL)
        .map(|provider| provider.id.clone())
        .collect();
    if !removed_legacy_local_provider_ids.is_empty() {
        settings
            .providers
            .retain(|provider| provider.base_url != LEGACY_APPLE_INTELLIGENCE_BASE_URL);
        let fallback = settings.providers.iter().find(|p| p.enabled).map(|p| {
            (
                p.id.clone(),
                p.enabled_models.first().cloned().unwrap_or_default(),
            )
        });

        if removed_legacy_local_provider_ids.contains(&settings.chat_provider_id) {
            if let Some((id, model)) = fallback.as_ref() {
                settings.chat_provider_id = id.clone();
                settings.chat_model = model.clone();
            } else {
                settings.chat_provider_id.clear();
                settings.chat_model.clear();
            }
        }
        if removed_legacy_local_provider_ids.contains(&settings.translator_provider_id) {
            if let Some((id, model)) = fallback.as_ref() {
                settings.translator_provider_id = id.clone();
                settings.translator_model = model.clone();
            } else {
                settings.translator_provider_id.clear();
                settings.translator_model.clear();
            }
        }
        if removed_legacy_local_provider_ids.contains(&settings.screenshot_translation.provider_id)
        {
            if let Some((id, model)) = fallback.as_ref() {
                settings.screenshot_translation.provider_id = id.clone();
                settings.screenshot_translation.model = model.clone();
            } else {
                settings.screenshot_translation.provider_id.clear();
                settings.screenshot_translation.model.clear();
            }
        }
        if !settings.lens.provider_id.is_empty()
            && removed_legacy_local_provider_ids.contains(&settings.lens.provider_id)
        {
            settings.lens.provider_id.clear();
            settings.lens.model.clear();
        }
        for selection in [
            &mut settings.default_models.chat,
            &mut settings.default_models.vision,
            &mut settings.default_models.video_analysis,
            &mut settings.default_models.title_summary,
            &mut settings.default_models.compression,
            &mut settings.default_models.image_generation,
            &mut settings.default_models.prompt_optimize,
        ] {
            if removed_legacy_local_provider_ids.contains(&selection.provider_id) {
                if let Some((id, model)) = fallback.as_ref() {
                    selection.provider_id = id.clone();
                    selection.model = model.clone();
                } else {
                    selection.provider_id.clear();
                    selection.model.clear();
                }
            }
        }
    }

    // Consume the legacy global override once; TASK is its sole new owner.
    let legacy_provider = std::mem::take(&mut settings.chat_tools.sub_agent_provider_id);
    let legacy_model = std::mem::take(&mut settings.chat_tools.sub_agent_model);
    if !legacy_provider.trim().is_empty() && !legacy_model.trim().is_empty() {
        settings
            .chat_tools
            .sub_agent_models
            .entry("task".into())
            .or_insert(SubAgentModelSelection {
                provider_id: legacy_provider,
                model: legacy_model,
                thinking_level: None,
            });
    }
    for selection in settings.chat_tools.sub_agent_models.values_mut() {
        selection.provider_id = selection.provider_id.trim().to_string();
        selection.model = selection.model.trim().to_string();
        let provider = settings
            .providers
            .iter()
            .find(|p| p.id == selection.provider_id && p.enabled);
        if provider.is_none() || selection.model.is_empty() {
            *selection = SubAgentModelSelection::default();
            continue;
        }
        if let Some(level) = selection.thinking_level.as_deref() {
            let levels = crate::chat::model_metadata::reasoning_efforts_for_model(
                provider,
                &selection.model,
            );
            if level != "off" && !levels.iter().any(|candidate| candidate == level) {
                selection.thinking_level = None;
            }
        }
    }

    let provider_exists = |id: &str| settings.providers.iter().any(|p| p.id == id);
    let provider_selectable = |id: &str| settings.providers.iter().any(|p| p.id == id && p.enabled);
    let first_selectable_provider = || settings.providers.iter().find(|p| p.enabled);

    // 2. 为空字段设置默认值
    if settings.translator_provider_id.is_empty() {
        if let Some(first) = first_selectable_provider() {
            settings.translator_provider_id = first.id.clone();
        }
    }
    if settings.screenshot_translation.provider_id.is_empty() {
        if let Some(first) = first_selectable_provider() {
            settings.screenshot_translation.provider_id = first.id.clone();
        }
    }
    if !settings.chat_provider_id.trim().is_empty()
        && settings.default_models.chat.provider_id.trim().is_empty()
    {
        settings.default_models.chat.provider_id = settings.chat_provider_id.clone();
        settings.default_models.chat.model = settings.chat_model.clone();
    }

    if settings.providers.is_empty() {
        settings.translator_provider_id.clear();
        settings.default_models = DefaultModelsConfig::default();
        settings.screenshot_translation.provider_id.clear();
        settings.lens.provider_id.clear();
        settings.chat_tools.sub_agent_models.clear();
    } else {
        if !provider_selectable(&settings.translator_provider_id) {
            if let Some(first) = first_selectable_provider() {
                settings.translator_provider_id = first.id.clone();
                if let Some(model) = first.enabled_models.first() {
                    settings.translator_model = model.clone();
                }
            } else if !provider_exists(&settings.translator_provider_id) {
                settings.translator_provider_id.clear();
                settings.translator_model.clear();
            }
        }
        if !provider_selectable(&settings.screenshot_translation.provider_id) {
            if let Some(first) = first_selectable_provider() {
                settings.screenshot_translation.provider_id = first.id.clone();
                if let Some(model) = first.enabled_models.first() {
                    settings.screenshot_translation.model = model.clone();
                }
            } else if !provider_exists(&settings.screenshot_translation.provider_id) {
                settings.screenshot_translation.provider_id.clear();
                settings.screenshot_translation.model.clear();
            }
        }
        // lens provider 可空（空时 call_vision_api 走 translator_provider_id fallback）；
        // 但若用户填了一个不存在或已禁用的，重置为空让其走 fallback。
        if !settings.lens.provider_id.is_empty()
            && (!provider_exists(&settings.lens.provider_id)
                || !provider_selectable(&settings.lens.provider_id))
        {
            settings.lens.provider_id.clear();
            settings.lens.model.clear();
        }

        sanitize_default_model_selection(&mut settings.default_models.chat, &settings.providers);
        sanitize_default_model_selection(&mut settings.default_models.vision, &settings.providers);
        sanitize_default_model_selection(
            &mut settings.default_models.video_analysis,
            &settings.providers,
        );
        sanitize_default_model_selection(
            &mut settings.default_models.title_summary,
            &settings.providers,
        );
        sanitize_default_model_selection(
            &mut settings.default_models.compression,
            &settings.providers,
        );
        sanitize_default_model_selection(
            &mut settings.default_models.image_generation,
            &settings.providers,
        );
        sanitize_default_model_selection(
            &mut settings.default_models.prompt_optimize,
            &settings.providers,
        );
        sanitize_default_model_selection(&mut settings.default_models.advisor, &settings.providers);
    }

    // 3. 确保当前使用的模型确实在该 provider 的 enabled_models 中。
    // enabled_models 可以为空：预设 provider 不再自带模型。
    for provider in &mut settings.providers {
        if settings.translator_provider_id == provider.id
            && !provider.enabled_models.contains(&settings.translator_model)
        {
            settings.translator_model =
                provider.enabled_models.first().cloned().unwrap_or_default();
        }
        if settings.screenshot_translation.provider_id == provider.id
            && !provider
                .enabled_models
                .contains(&settings.screenshot_translation.model)
        {
            settings.screenshot_translation.model =
                provider.enabled_models.first().cloned().unwrap_or_default();
        }
        if !settings.lens.provider_id.is_empty()
            && settings.lens.provider_id == provider.id
            && !settings.lens.model.is_empty()
            && !provider.enabled_models.contains(&settings.lens.model)
        {
            settings.lens.model = provider.enabled_models.first().cloned().unwrap_or_default();
        }
    }

    sync_legacy_chat_model_fields(&mut settings);

    // 4. 规范化快捷键字符串
    settings.hotkey = normalize_hotkey(&settings.hotkey);
    settings.chat_hotkey = normalize_hotkey(&settings.chat_hotkey);
    settings.close_chat_hotkey = normalize_hotkey(&settings.close_chat_hotkey);
    settings.screenshot_translation.hotkey =
        normalize_hotkey(&settings.screenshot_translation.hotkey);
    settings.screenshot_translation.text_hotkey =
        normalize_hotkey(&settings.screenshot_translation.text_hotkey);
    settings.screenshot_translation.replace_hotkey =
        normalize_hotkey(&settings.screenshot_translation.replace_hotkey);
    settings.lens.hotkey = normalize_hotkey(&settings.lens.hotkey);
    settings.screenshot_annotate.hotkey = normalize_hotkey(&settings.screenshot_annotate.hotkey);

    // 规范化提示词（去除首尾空白，空值转为 None）
    settings.translator_prompt = normalize_optional_prompt(settings.translator_prompt.take());
    settings.screenshot_translation.prompt =
        normalize_optional_prompt(settings.screenshot_translation.prompt.take());
    settings.screenshot_translation.text_prompt =
        normalize_optional_prompt(settings.screenshot_translation.text_prompt.take());
    settings.screenshot_translation.replace_prompt =
        normalize_optional_prompt(settings.screenshot_translation.replace_prompt.take());
    // 翻译卡宽度单一真源：import / 手改 settings.json 也在此兜底到 360–720，
    // 与 set_translate_card_size 命令、设置页输入框、Lens 缩放 clamp 同域。
    settings.screenshot_translation.card_width =
        settings.screenshot_translation.card_width.clamp(360, 720);

    // 5. 其他字段验证。自定义主题先规范化，themeColor 才能指向仍然存在的 id。
    if !matches!(settings.theme.as_str(), "system" | "light" | "dark") {
        settings.theme = default_theme();
    }
    sanitize_custom_themes(&mut settings.custom_themes);
    settings.theme_color = canonicalize_theme_color(&settings.theme_color, &settings.custom_themes);
    settings.ui_font_scale = if settings.ui_font_scale.is_finite() {
        settings.ui_font_scale.clamp(0.8, 1.4)
    } else {
        default_ui_font_scale()
    };
    if settings.lens.message_order != "asc" && settings.lens.message_order != "desc" {
        settings.lens.message_order = "asc".to_string();
    }
    settings.lens.web_search.tavily_api_key =
        settings.lens.web_search.tavily_api_key.trim().to_string();
    settings.lens.web_search.exa_api_key = settings.lens.web_search.exa_api_key.trim().to_string();
    settings.lens.web_search.ollama_api_key =
        settings.lens.web_search.ollama_api_key.trim().to_string();
    settings.lens.web_search.grok_api_key =
        settings.lens.web_search.grok_api_key.trim().to_string();
    settings.lens.web_search.deepseek_api_key =
        settings.lens.web_search.deepseek_api_key.trim().to_string();
    settings.lens.web_search.brave_api_key =
        settings.lens.web_search.brave_api_key.trim().to_string();
    settings.lens.web_search.serper_api_key =
        settings.lens.web_search.serper_api_key.trim().to_string();
    settings.lens.web_search.bocha_api_key =
        settings.lens.web_search.bocha_api_key.trim().to_string();
    settings.lens.web_search.zhipu_api_key =
        settings.lens.web_search.zhipu_api_key.trim().to_string();
    settings.lens.web_search.tinyfish_api_key =
        settings.lens.web_search.tinyfish_api_key.trim().to_string();
    settings.lens.web_search.tinyfish_mcp_url = {
        let trimmed = settings.lens.web_search.tinyfish_mcp_url.trim();
        if trimmed.is_empty() {
            default_tinyfish_mcp_url()
        } else {
            trimmed.to_string()
        }
    };
    if let Some(auth) = settings.lens.web_search.tinyfish_mcp_auth.as_mut() {
        auth.access_token = auth.access_token.trim().to_string();
    }
    if settings
        .lens
        .web_search
        .tinyfish_mcp_auth
        .as_ref()
        .is_some_and(|auth| auth.access_token.is_empty())
    {
        settings.lens.web_search.tinyfish_mcp_auth = None;
    }
    settings.lens.web_search.searxng_base_url =
        settings.lens.web_search.searxng_base_url.trim().to_string();
    settings.lens.web_search.grok_model = {
        let trimmed = settings.lens.web_search.grok_model.trim();
        if trimmed.is_empty() {
            default_grok_model()
        } else {
            trimmed.to_string()
        }
    };
    settings.lens.web_search.grok_base_url = {
        let trimmed = settings.lens.web_search.grok_base_url.trim();
        if trimmed.is_empty() {
            default_grok_base_url()
        } else {
            trimmed.to_string()
        }
    };
    if settings
        .lens
        .web_search
        .grok_system_prompt
        .trim()
        .is_empty()
    {
        settings.lens.web_search.grok_system_prompt = default_grok_system_prompt();
    }
    settings.lens.web_search.deepseek_base_url = {
        let trimmed = settings.lens.web_search.deepseek_base_url.trim();
        if trimmed.is_empty() {
            default_deepseek_base_url()
        } else {
            trimmed.to_string()
        }
    };
    settings.lens.web_search.exa_mcp_url = {
        let trimmed = settings.lens.web_search.exa_mcp_url.trim();
        if trimmed.is_empty() {
            default_exa_mcp_url()
        } else {
            trimmed.to_string()
        }
    };
    // 未知/占位服务商回退到 Tavily，避免选中尚未接入的源导致搜索直接报错。
    if matches!(
        settings.lens.web_search.provider,
        WebSearchProvider::Unknown
    ) {
        settings.lens.web_search.provider = WebSearchProvider::Tavily;
    }
    // Fetch 源未设 = 跟随搜索。未知、或没有 URL fetch 接口的源清掉，避免配了却永远走直连。
    if settings
        .lens
        .web_search
        .fetch_provider
        .is_some_and(|provider| {
            matches!(provider, WebSearchProvider::Unknown)
                || !config::provider_supports_fetch(provider)
                || provider == settings.lens.web_search.provider
        })
    {
        settings.lens.web_search.fetch_provider = None;
    }
    settings.lens.web_search.max_results = settings.lens.web_search.max_results.clamp(1, 10);
    if !matches!(
        settings.lens.web_search.search_depth.as_str(),
        "ultra-fast" | "fast" | "basic" | "advanced"
    ) {
        settings.lens.web_search.search_depth = default_web_search_depth();
    }

    // ponytail: 流式输出 / 思考模式的设置项已从 UI 移除，恒为开启（老配置里的 false 在此归位）
    settings.chat.stream_enabled = true;
    settings.chat.thinking_enabled = true;

    if !settings.chat_behavior_migrated_from_lens {
        if settings.lens.default_language.trim().is_empty() {
            // keep chat.default_language empty → inherit chain unchanged
        } else {
            settings.chat.default_language = settings.lens.default_language.clone();
        }
        settings.chat_behavior_migrated_from_lens = true;
    }
    if !matches!(
        settings.chat.default_language.trim(),
        "" | "zh" | "zh-Hant" | "en"
    ) {
        settings.chat.default_language.clear();
    }
    settings.chat.max_output_tokens = clamp_chat_max_output_tokens(settings.chat.max_output_tokens);
    settings.chat.system_prompt = settings.chat.system_prompt.trim().to_string();

    settings.chat_tools.tool_timeout_ms = settings
        .chat_tools
        .tool_timeout_ms
        .clamp(CHAT_TOOL_MIN_TIMEOUT_MS, CHAT_TOOL_MAX_TIMEOUT_MS);
    settings.chat_tools.sub_agent_concurrency = settings.chat_tools.sub_agent_concurrency.clamp(
        tools::SUB_AGENT_CONCURRENCY_MIN,
        tools::SUB_AGENT_CONCURRENCY_MAX,
    );
    settings.chat_tools.mcp_idle_timeout_ms = settings
        .chat_tools
        .mcp_idle_timeout_ms
        .clamp(MCP_IDLE_TIMEOUT_MIN_MS, MCP_IDLE_TIMEOUT_MAX_MS);
    // 工具输出截断：None（旧的"不截断"）归一到默认值，Some 值钳到合法区间。
    // 旧逻辑在此无条件置 None（等于永不截断 → 上下文撑爆主因），现改为始终保底截断。
    settings.chat_tools.max_tool_output_chars = Some(
        settings
            .chat_tools
            .max_tool_output_chars
            .unwrap_or(DEFAULT_MAX_TOOL_OUTPUT_CHARS)
            .clamp(CHAT_TOOL_MIN_OUTPUT_CHARS, CHAT_TOOL_MAX_OUTPUT_CHARS),
    );
    if !matches!(
        settings.chat_tools.approval_policy.trim(),
        "readonly_auto_sensitive_confirm" | "always_confirm" | "auto"
    ) {
        settings.chat_tools.approval_policy = default_chat_approval_policy();
    }
    sanitize_hooks(&mut settings.chat_tools.hooks);
    // One-shot green-light migration: bring a pre-green-light install (native
    // tools defaulted OFF) to the capability baseline. Approval is never weakened. Idempotent
    // via `chat_tools_greenlit_v1` so a user who later turns a tool back off, or
    // picks a stricter policy, is never re-flipped. Explicit approval choices survive.
    if !settings.chat_tools_greenlit_v1 {
        let native = &mut settings.chat_tools.native_tools;
        native.read_file = true;
        native.write_file = true;
        native.edit_file = true;
        native.run_command = true;
        native.web_fetch = true;
        native.web_search = true;
        settings.chat_tools_greenlit_v1 = true;
    }
    settings.chat_tools.skill_scan_paths = settings
        .chat_tools
        .skill_scan_paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect();
    if !matches!(
        settings.chat_tools.skill_fallback_mode.trim(),
        "progressive" | "skill_md_only" | "legacy_full_body"
    ) {
        settings.chat_tools.skill_fallback_mode = default_skill_fallback_mode();
    }
    settings.chat_tools.disabled_skill_ids = settings
        .chat_tools
        .disabled_skill_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    settings.chat_tools.native_tools.workspace_roots = settings
        .chat_tools
        .native_tools
        .workspace_roots
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect();
    let mut seen_roots = std::collections::HashSet::new();
    settings
        .chat_tools
        .native_tools
        .workspace_roots
        .retain(|path| seen_roots.insert(path.clone()));
    settings.chat_tools.native_tools.working_directory = settings
        .chat_tools
        .native_tools
        .working_directory
        .trim()
        .to_string();
    if settings
        .chat_tools
        .native_tools
        .working_directory
        .is_empty()
    {
        settings.chat_tools.native_tools.working_directory = settings
            .chat_tools
            .native_tools
            .workspace_roots
            .first()
            .cloned()
            .unwrap_or_else(default_chat_working_directory);
    }
    // Persist only the new single-directory setting after legacy migration.
    settings.chat_tools.native_tools.workspace_roots.clear();
    // Cua now runs through its Skill + CLI. Retire only Kivio-managed MCP
    // entries; preserve the user's disabled state and unrelated MCP servers.
    let mut cua_enabled = None;
    settings.chat_tools.servers.retain(|server| {
        // Retire the managed integration on load/import/save, including renamed
        // entries. Independently configured user servers remain user-owned.
        if server.id.trim() == "plugin-officecli"
            || server.connector_id.as_deref().map(str::trim) == Some("plugin:officecli")
        {
            return false;
        }
        let managed_cua = tools::is_cua_mcp_server_id(server.id.trim())
            || matches!(
                server.connector_id.as_deref().map(str::trim),
                Some("computer-control:cua" | "plugin:cua-driver")
            );
        if managed_cua {
            cua_enabled = Some(cua_enabled.unwrap_or(false) || server.enabled);
        }
        !managed_cua
    });
    if cua_enabled == Some(false)
        && !settings
            .chat_tools
            .disabled_skill_ids
            .iter()
            .any(|id| id == "cua-driver")
    {
        settings
            .chat_tools
            .disabled_skill_ids
            .push("cua-driver".to_string());
    }
    for server in &mut settings.chat_tools.servers {
        server.id = server.id.trim().to_string();
        if server.id.is_empty() {
            server.id = format!("mcp-{}", uuid::Uuid::new_v4());
        }
        server.name = server.name.trim().to_string();
        if server.name.is_empty() {
            server.name = server.id.clone();
        }
        server.transport = server.transport.trim().to_ascii_lowercase();
        if server.transport == "http" || server.transport == "sse" {
            server.transport = "streamable_http".to_string();
        }
        if server.transport != "stdio" && server.transport != "streamable_http" {
            server.transport = "stdio".to_string();
        }
        server.url = server.url.trim().to_string();
        server.command = server.command.trim().to_string();
        server.args = server
            .args
            .iter()
            .map(|arg| arg.trim().to_string())
            .filter(|arg| !arg.is_empty())
            .collect();
        server.env = server
            .env
            .iter()
            .filter_map(|(key, value)| {
                let key = key.trim();
                if key.is_empty() {
                    None
                } else {
                    Some((key.to_string(), value.clone()))
                }
            })
            .collect();
        server.headers = server
            .headers
            .iter()
            .filter_map(|(key, value)| {
                let key = key.trim();
                if key.is_empty() {
                    None
                } else {
                    Some((key.to_string(), value.trim().to_string()))
                }
            })
            .collect();
        server.cwd = server.cwd.take().and_then(|cwd| {
            let trimmed = cwd.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        server.enabled_tools = server
            .enabled_tools
            .iter()
            .map(|tool| tool.trim().to_string())
            .filter(|tool| !tool.is_empty())
            .collect();
        // 连接器 id 去空白；空串归一为 None，使其退回普通 MCP server。
        server.connector_id = server.connector_id.take().and_then(|cid| {
            let trimmed = cid.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
    }
    let mut seen_server_ids = std::collections::HashSet::new();
    settings
        .chat_tools
        .servers
        .retain(|server| seen_server_ids.insert(server.id.clone()));

    // 清理归档目录路径（去除首尾空白）
    settings.image_archive_path = settings.image_archive_path.trim().to_string();

    settings.retry_attempts = clamp_retry_attempts(settings.retry_attempts);

    // 系统 OCR 依赖平台本地 OCR 能力（macOS Apple Vision / Windows.Media.Ocr）。其它平台
    // 同步来的旧配置必须关闭，否则截图翻译会误入不可用分支。
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        settings.screenshot_translation.use_system_ocr = false;
    }

    // OCR 引擎模式迁移（vNext+）：
    // 1. 反序列化兜底变体 OcrMode::Legacy（如旧版 "tesseract" 字符串）→ RapidOcr，
    //    保留用户此前选择离线 OCR 的隐私边界；模型未下载时由前端引导下载。
    // 2. 若 ocr_mode 缺省（老版本数据），按 use_system_ocr 反推：
    //    true→System，false→CloudVision
    // 3. Linux 不支持 System / RapidOcr，强制落回 CloudVision
    settings.screenshot_translation.ocr_mode =
        Some(settings.screenshot_translation.resolved_ocr_mode());
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if matches!(
            settings.screenshot_translation.ocr_mode,
            Some(OcrMode::System) | Some(OcrMode::RapidOcr)
        ) {
            settings.screenshot_translation.ocr_mode = Some(OcrMode::CloudVision);
        }
    }

    settings.onboarding_status = normalize_onboarding_status(&settings);

    sanitize_external_cli_agents(&mut settings.chat.external_cli_agents);

    settings
}

/// settings.json 是用户可手改的文件：环境变量名/模型 id 空白或全空格会被原样塞进子进程环境，
/// 表现成难查的「CLI 启动即报错」。这里统一 trim 并丢掉空条目，未知 agent id 的整条配置也丢掉。
fn sanitize_external_cli_agents(
    agents: &mut std::collections::HashMap<String, ExternalCliAgentConfig>,
) {
    agents.retain(|id, cfg| {
        if crate::external_agents::registry::get_agent_def(id).is_none() {
            return false;
        }
        cfg.path = cfg.path.trim().to_string();
        for pair in cfg.env.iter_mut() {
            pair.key = pair.key.trim().to_string();
            pair.value = pair.value.trim().to_string();
        }
        cfg.env.retain(|pair| !pair.key.is_empty());
        for model in cfg.custom_models.iter_mut() {
            model.id = model.id.trim().to_string();
            model.label = model.label.trim().to_string();
        }
        cfg.custom_models.retain(|model| !model.id.is_empty());
        for provider in cfg.providers.iter_mut() {
            provider.id = provider.id.trim().to_string();
            provider.name = provider.name.trim().to_string();
            for pair in provider.env.iter_mut() {
                pair.key = pair.key.trim().to_string();
                pair.value = pair.value.trim().to_string();
            }
            provider.env.retain(|pair| !pair.key.is_empty());
        }
        cfg.providers
            .retain(|provider| !provider.id.is_empty() && !provider.name.is_empty());
        // 悬空的 current_provider（供应商被删了 / 手改错了）必须归零：留着会让注入层
        // 找不到条目而**静默什么都不注入**，用户看到的是「选了供应商但没生效」。
        cfg.current_provider = cfg.current_provider.trim().to_string();
        if !cfg
            .providers
            .iter()
            .any(|provider| provider.id == cfg.current_provider && !provider.disabled)
        {
            cfg.current_provider = String::new();
        }
        true
    });
}

fn onboarding_status_is_set(raw: &str) -> bool {
    matches!(raw.trim(), "pending" | "completed" | "skipped")
}

fn provider_has_usable_config(provider: &ModelProvider) -> bool {
    provider.enabled && provider.authentication_ready() && !provider.enabled_models.is_empty()
}

fn settings_has_usable_provider_config(settings: &Settings) -> bool {
    settings.providers.iter().any(provider_has_usable_config)
}

fn normalize_onboarding_status(settings: &Settings) -> String {
    let raw = settings.onboarding_status.trim();
    if onboarding_status_is_set(raw) {
        return raw.to_string();
    }
    if settings_has_usable_provider_config(settings) {
        "completed".to_string()
    } else {
        "pending".to_string()
    }
}

fn normalize_optional_prompt(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/**
 * 规范化快捷键字符串：去除各部分首尾空白并过滤空部分
 */
fn normalize_hotkey(value: &str) -> String {
    value
        .split('+')
        .map(|part| {
            let trimmed = part.trim();
            match trimmed.to_lowercase().as_str() {
                "cmd" | "command" | "commandorcontrol" => "CommandOrControl".to_string(),
                "ctrl" | "control" => "Control".to_string(),
                "opt" | "option" | "alt" => "Alt".to_string(),
                "shift" => "Shift".to_string(),
                "super" | "meta" => "Super".to_string(),
                "plus" => "Plus".to_string(),
                _ => trimmed.to_string(),
            }
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("+")
}


fn clamp_retry_attempts(value: u8) -> u8 {
    value.clamp(1, 11)
}
