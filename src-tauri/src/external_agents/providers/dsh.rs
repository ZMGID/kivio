//! DeepSeek Harness uses its native SDK stream through a Kivio-owned profile.
//! Rust owns installation, routing and protocol delivery; the in-process bridge
//! composes native agents and exposes cancellation, steering and user interaction.

use super::super::types::{CapabilitySupport, RewindStrategy};
use super::super::types::{
    ExternalCliSlashCommand, PromptInputFormat, RuntimeAgentDef, RuntimeBuildOptions,
    RuntimeContext, SlashStrategy, StreamFormat,
};

/// Fallback `/` menu before a live kivio dsh session has reported
/// `session/commands`. Official web-only entries that are not registered in
/// this headless profile stay off the fallback; discovery lists whatever
/// `ctx.commands.list` actually returns.
const DSH_SLASH_COMMANDS: &[(&str, &str, Option<&str>)] = &[
    ("compact", "Compact older conversation history", None),
    (
        "feedback",
        "record feedback about this session",
        Some("<text>"),
    ),
    (
        "goal",
        "set or view the goal for a long-running task",
        Some("[<objective>|clear|edit <objective>|pause|resume]"),
    ),
    (
        "plan",
        "enter plan mode or submit a planning message",
        Some("[off|<message>]"),
    ),
];

pub fn builtin_slash_commands() -> Vec<ExternalCliSlashCommand> {
    DSH_SLASH_COMMANDS
        .iter()
        .map(|(name, description, hint)| ExternalCliSlashCommand {
            slash: format!("/{name}"),
            name: (*name).to_string(),
            description: Some((*description).to_string()),
            argument_hint: hint.map(|value| (*value).to_string()),
        })
        .collect()
}

/// dsh 自带 DeepSeek 适配器（`deepseek-official` 路由）的三个模型。真实列表由
/// `detection::read_dsh_settings_models` 从 `~/.dsh/settings.yaml` 读出（含用户在
/// `llm-pi-ai.providers` 里配的中转路由）；这张表只是读不到时的兜底，前端标「默认列表」。
///
/// `deepseek-v4-flash-vision-exp` 是 0.1.1 起的多模态目录项（`inputModalities: [text, image]`）。
/// 能力写在 dsh 自己的 catalog 里，Kivio 不往 `llm-deepseek` 补这份声明——Flash / Pro
/// 仍是纯文本，硬开 `inputModalities` 会 400。
const FALLBACK_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("deepseek-v4-flash", "DeepSeek-V4-Flash"),
    ("deepseek-v4-pro", "DeepSeek-V4-Pro"),
    (
        "deepseek-v4-flash-vision-exp",
        "DeepSeek-V4-Flash-Vision-Exp",
    ),
];

/// 官方 DeepSeek 路由的兜底档位：`off | low | high | max`（rc.7 起有 `low`）。
/// 第三方 `llm-pi-ai` 模型
/// 的可选档位由 `detection::parse_dsh_settings_models` 按 `reasoningEfforts` 填
/// `reasoning_by_model`；选中后写进 profile 的 `llm-pi-ai.providers.<route>.reasoning`。
const REASONING: &[(&str, &str)] = &[
    ("default", "Default"),
    ("off", "Off"),
    ("low", "Low"),
    ("high", "High"),
    ("max", "Max"),
];

/// `dsh --profile <name>`。
///
/// profile 名固定（`dsh_profile::KIVIO_PROFILE`）：它是 Kivio 私有的一份装配，与用户自己的
/// `web` / `tui` profile 并存互不影响。模型/档位都不进 argv，见模块注释。
pub fn build_dsh_args(
    _ctx: &RuntimeContext,
    _options: &RuntimeBuildOptions,
    _prompt: Option<&str>,
) -> Vec<String> {
    vec![
        "--profile".to_string(),
        crate::external_agents::dsh_profile::KIVIO_PROFILE.to_string(),
    ]
}

pub const DSH_AGENT_DEF: RuntimeAgentDef = RuntimeAgentDef {
    id: "dsh",
    name: "DeepSeek Harness",
    bin: "dsh",
    fallback_bins: &[],
    version_args: &["--version"],
    // `dsh` 没有登录态概念：凭据是 `~/.dsh/.credentials.yaml` / `.env` / 环境变量里的
    // `DEEPSEEK_API_KEY`，没有任何「查询登录状态」的子命令。探测认证只会白起一个进程。
    auth_probe_args: None,
    fallback_models: FALLBACK_MODELS,
    reasoning_options: REASONING,
    sandbox_options: &[
        ("read-only", "只读"),
        ("workspace-write", "工作区写 (默认)"),
        ("danger-full-access", "完全"),
    ],
    // 模型列表读配置文件，不起进程（见 `detection::probe_models` 的 dsh 分支）。
    list_models_args: None,
    list_models_timeout_secs: None,
    models_from_stderr: false,
    model_probe: Some(super::super::types::ModelProbeStrategy::DshSettings),
    model_probe_args: None,
    current_config: super::super::types::CurrentConfigStrategy::None,
    provider_profile: super::super::types::ProviderProfileStrategy::Dsh,
    native_providers: super::super::types::NativeProviderStrategy::DshSettings,
    context_window: super::super::types::ContextWindowStrategy::Generic,
    usage_fallback: super::super::types::UsageFallbackStrategy::None,
    error_policy: super::super::types::AgentErrorPolicy::SETTINGS_API_KEY,
    launch: super::super::types::AgentLaunchPolicy::DEFAULT,
    instructions_via_launch_flag: false,
    compact_prompt: Some("/compact"),
    install: super::super::types::AgentInstallSpec {
        npm_package: Some("@deepseek-ai/dsh"),
        npm_install_args: &[],
        pypi_package: None,
        script_unix: None,
        script_windows: None,
        update: super::super::types::UpdateStrategy::ManagedPackage {
            package: "@deepseek-ai/dsh",
            brew_formula: "dsh",
        },
        latest_version: super::super::types::LatestVersionStrategy::Registry,
        docs: "https://github.com/deepseek-ai/deepseek-harness",
        config_dir: Some(".dsh"),
        config_dir_env: Some("DSH_HOME"),
        requires_pnpm: true,
        post_install: super::super::types::PostInstallStrategy::DshProfile,
    },
    import: super::super::types::AgentImportPolicy::NONE,
    run: super::super::types::AgentRunPolicy::STANDARD,
    // 斜杠菜单：先走内建兜底，连上后 `session/commands` 覆盖缓存。
    slash_strategy: SlashStrategy::Dsh,
    // 遥测默认关：任何非空值都算关（上游的隐私开关刻意「误关优于误开」）。用户想开就
    // 在 `~/.dsh/.env` 里自己设 —— 那份 env 由 dsh 自己加载，覆盖不到这里。
    env: &[("DSH_TELEMETRY_DISABLED", "1")],
    max_prompt_arg_bytes: None,
    // prompt 走 JSON-RPC 的 `contentBlocks`，既不进 argv 也不进裸 stdin。
    prompt_via_stdin: false,
    prompt_input_format: PromptInputFormat::Text,
    stream_format: StreamFormat::DshJsonRpc,
    // 原生 session id 由 Kivio bridge 的 `session/open` 创建/恢复，不进 argv；因此这里仍不是
    // `resumes_session_via_cli`（该字段只描述 `--resume` 一类 CLI 参数形状）。
    resumes_session_via_cli: false,
    // `session/prompt` 的官方 image 块是 `{type:"image", attachment:{attachmentId,…}}`。
    // Kivio 先发 base64，bridge 经 `ctx.attachments.saveImage` 落库后再改写成引用。
    image_input: CapabilitySupport::Native,
    // Bridge 补 `session/steer` → `agent.steer()`（next-step inbox，当前轮下一步）。
    // 官方 `session/prompt` → `agent.followup()`（下一轮 FIFO）。两套都接。
    steering: CapabilitySupport::Native,
    follow_up: CapabilitySupport::Native,
    rewind: RewindStrategy::Replay,
    account: None,
    image_mime_whitelist: &["image/jpeg", "image/png", "image/gif", "image/webp"],
    build_args: build_dsh_args,
};

fn dsh_provider_fingerprint_for(
    config: Option<&crate::settings::ExternalCliAgentConfig>,
) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match config {
        Some(config) => serde_json::to_string(&(&config.current_provider, &config.providers))
            .unwrap_or_else(|_| config.current_provider.clone())
            .hash(&mut hasher),
        None => "cli-default".hash(&mut hasher),
    }
    format!("{:016x}", hasher.finish())
}

fn dsh_provider_fingerprint() -> String {
    let config = crate::external_agents::overrides::agent_config("dsh");
    dsh_provider_fingerprint_for(config.as_ref())
}

pub(crate) fn launch_config(
    model: Option<&str>,
    reasoning: Option<&str>,
    sandbox: Option<&str>,
    preset: Option<&str>,
) -> crate::external_agents::session::live::LaunchConfig {
    use crate::external_agents::session::live::LaunchConfig;

    LaunchConfig {
        identity: None,
        flags: format!(
            "{}|{}|{}|{}|{}",
            model.unwrap_or_default(),
            reasoning.unwrap_or_default(),
            sandbox.unwrap_or_default(),
            crate::external_agents::dsh_profile::normalize_agent_preset(preset),
            dsh_provider_fingerprint()
        ),
        // dsh 的会话级指令在首轮正文里，不是启动配置；指令变化不需要为了它单独重连。
        instructions: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_fingerprint_tracks_provider_settings() {
        let config_a = crate::settings::ExternalCliAgentConfig {
            providers: vec![crate::settings::ExternalCliProvider {
                id: "provider-a".to_string(),
                config_json: "{\"baseURL\":\"https://a.example/v1\"}".to_string(),
                ..Default::default()
            }],
            current_provider: "provider-a".to_string(),
            ..Default::default()
        };
        let config_b = crate::settings::ExternalCliAgentConfig {
            providers: vec![crate::settings::ExternalCliProvider {
                id: "provider-b".to_string(),
                config_json: "{\"baseURL\":\"https://b.example/v1\"}".to_string(),
                ..Default::default()
            }],
            current_provider: "provider-b".to_string(),
            ..Default::default()
        };
        assert_ne!(
            dsh_provider_fingerprint_for(Some(&config_a)),
            dsh_provider_fingerprint_for(Some(&config_b))
        );
        let mut config_disabled = config_a.clone();
        config_disabled.providers[0].disabled = true;
        assert_ne!(
            dsh_provider_fingerprint_for(Some(&config_a)),
            dsh_provider_fingerprint_for(Some(&config_disabled))
        );
        assert_ne!(
            dsh_provider_fingerprint_for(None),
            dsh_provider_fingerprint_for(Some(&config_a))
        );
    }

    fn ctx() -> RuntimeContext {
        RuntimeContext {
            extra_allowed_dirs: vec![],
            resume_session_id: None,
            new_session_id: None,
            include_partial_messages: false,
        }
    }

    /// 模型与推理档位**不进 argv**：模型是 `initialize` 的 RPC 参数，档位是 profile patch 里的
    /// `llm-deepseek.reasoningEffort`。硬塞进 argv 会被 dsh 的启动器当作「要转交给 app 的
    /// 内部参数」原样传下去（`args.ts` 的 passthrough 语义），既不生效也不报错。
    #[test]
    fn model_and_reasoning_never_reach_argv() {
        let args = (DSH_AGENT_DEF.build_args)(
            &ctx(),
            &RuntimeBuildOptions {
                model: Some("deepseek-v4-pro".to_string()),
                reasoning: Some("max".to_string()),
                sandbox: Some("danger-full-access".to_string()),
            },
            Some("hello"),
        );
        assert_eq!(args, vec!["--profile".to_string(), "kivio".to_string()]);
    }

    /// 这条线的能力边界是**实测**出来的，不是保守估计。改任一项之前先按模块注释复验协议。
    #[test]
    fn declares_the_measured_protocol_limits() {
        assert!(matches!(
            DSH_AGENT_DEF.stream_format,
            StreamFormat::DshJsonRpc
        ));
        assert!(matches!(DSH_AGENT_DEF.slash_strategy, SlashStrategy::Dsh));
        assert_eq!(DSH_AGENT_DEF.steering, CapabilitySupport::Native);
        assert_eq!(DSH_AGENT_DEF.follow_up, CapabilitySupport::Native);
        assert_eq!(DSH_AGENT_DEF.image_input, CapabilitySupport::Native);
        assert_eq!(
            DSH_AGENT_DEF.image_mime_whitelist,
            &["image/jpeg", "image/png", "image/gif", "image/webp"] as &[&str]
        );
        assert!(!DSH_AGENT_DEF.resumes_session_via_cli);
        assert!(matches!(
            DSH_AGENT_DEF.model_probe,
            Some(crate::external_agents::types::ModelProbeStrategy::DshSettings)
        ));
        assert!(DSH_AGENT_DEF.auth_probe_args.is_none());
    }

    #[test]
    fn lists_the_official_slash_menu() {
        let commands = builtin_slash_commands();
        let names: Vec<&str> = commands
            .iter()
            .map(|command| command.name.as_str())
            .collect();
        assert_eq!(names, ["compact", "feedback", "goal", "plan"]);
        assert!(commands
            .iter()
            .all(|command| command.slash.starts_with('/')));
    }
}
