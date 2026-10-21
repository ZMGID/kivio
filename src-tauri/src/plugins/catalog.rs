//! 内置插件目录：广场条目 + 安装规范 + 启用后注入的 MCP / 提示。
//!
//! ego lite 在启用时从仓库下载 Skill。OfficeCLI 已退役，旧技能 ID 仅用于阻止残留技能重新接入。

/// 官方 README 里的安装命令（按平台）。只给后端自动执行，不展示给用户。
#[derive(Debug, Clone)]
pub struct PluginInstallCommand {
    /// windows | unix | macos | linux | any
    pub platform: &'static str,
    pub command: &'static str,
}

/// 插件附带的 stdio MCP 规格：启用时挂到 chatTools.servers，关闭时禁用/断连。
#[derive(Debug, Clone)]
pub struct PluginMcpSpec {
    /// 传给二进制的参数，如 `["mcp"]` → `officecli mcp`（stdio JSON-RPC）
    pub args: &'static [&'static str],
}

#[derive(Debug, Clone)]
pub struct CatalogPlugin {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub binary: &'static str,
    pub tags: &'static [&'static str],
    pub homepage: &'static str,
    pub repo: &'static str,
    /// 官方安装器常用落点（支持 `%LOCALAPPDATA%` 等环境变量）；PATH 未刷新时仍可检测到
    pub known_binary_paths: &'static [&'static str],
    /// 官方 README 原文 URL（raw）；权威安装/用法以 README 为准
    pub readme_urls: &'static [&'static str],
    /// GitHub README 里的官方安装命令（按平台）
    pub install_commands: &'static [PluginInstallCommand],
    /// 短 system 提示（仅 enabled 时注入）
    pub system_hint: &'static str,
    /// 本插件拥有的 skill id 列表（启用才可见）
    pub skill_ids: &'static [&'static str],
    /// 附属 Skill 正文（启用时写入插件目录；官方共享目录型插件保持为空）
    pub skill_md: &'static str,
    /// 可选：从该 GitHub 仓库 / 直链 zip 下载附属 Skill（启用时下载进插件 skill 目录，随上游更新）。
    /// 与 `skill_md` 二选一；两者皆空时走官方 `~/.agents/skills`（见 [`CatalogPlugin::uses_shared_skill_dirs`]）。
    pub skill_download_url: Option<&'static str>,
    /// 可选 MCP：启用时自动注册 stdio server
    pub mcp: Option<PluginMcpSpec>,
    /// **Kivio 安装契约**（薄层）：流程/约束/验收；具体命令以 README 为准，勿与 README 冲突
    pub install_doc: &'static str,
}

impl CatalogPlugin {
    /// 官方安装器把 Skill 写到 `~/.agents/skills` 等共享目录；Kivio 直接扫描，启用时不必再拷贝。
    pub fn uses_shared_skill_dirs(&self) -> bool {
        !self.skill_ids.is_empty()
            && self.skill_md.trim().is_empty()
            && self.skill_download_url.is_none()
    }

    /// 当前系统对应的 GitHub README 安装命令。
    pub fn host_install_command(&self) -> Option<&'static str> {
        let mut unix = None;
        for cmd in self.install_commands {
            match cmd.platform {
                "any" => return Some(cmd.command),
                "windows" if cfg!(windows) => return Some(cmd.command),
                "macos" if cfg!(target_os = "macos") => return Some(cmd.command),
                "linux" if cfg!(target_os = "linux") => return Some(cmd.command),
                "unix" if cfg!(unix) => unix = Some(cmd.command),
                _ => {}
            }
        }
        unix
    }
}

pub const PLUGIN_CATALOG: &[CatalogPlugin] = &[CatalogPlugin {
    id: "ego-lite",
    name: "ego lite",
    description: "面向 AI Agent 的 Chromium 浏览器（macOS）。Agent 在独立空间复用你的登录态，用于打开网页、填表、点击、截图、抓取、Web 测试等。附带 ego-browser Skill（由 Kivio 从仓库自动下载）。",
    binary: "ego-browser",
    tags: &["Browser", "Automation", "Skill", "macOS"],
    homepage: "https://lite.ego.app/",
    repo: "https://github.com/citrolabs/ego-lite",
    // onboarding 后注册到用户 bin；PATH 未刷新时按这些路径兜底检测。
    // macOS 无 %USERPROFILE% 环境变量，须用展开器支持的 $HOME。
    known_binary_paths: &[
        "$HOME/.local/bin/ego-browser",
        "/usr/local/bin/ego-browser",
        "/opt/homebrew/bin/ego-browser",
    ],
    readme_urls: &["https://raw.githubusercontent.com/citrolabs/ego-lite/main/README.md"],
    install_commands: &[PluginInstallCommand {
        platform: "macos",
        command: "npx skills add citrolabs/ego-lite",
    }],
    system_hint: "\
### ego lite\n\
**Role.** Real Chromium browser automation for interactive web tasks — open pages, fill forms, click, screenshot, scrape, log in, test web apps.\n\
Prefer the `ego-browser` skill over web_fetch / built-in browsing whenever the task needs real page interaction.\n\
Activate the `ego-browser` skill, then run browser work via run_command as `ego-browser nodejs <<'EOF' … EOF` (default one invocation per task). Do NOT import Playwright or launch another browser.",
    skill_ids: &["ego-browser"],
    skill_md: "", // 由 Kivio 从 skill_download_url 下载
    skill_download_url: Some("https://github.com/citrolabs/ego-lite"),
    mcp: None,
    install_doc: EGO_LITE_INSTALL_DOC,
}];

/// `officecli load_skill` / skills install 的完整集合（CLI 子名 → frontmatter skill id）。
/// base `officecli` 单独从 skills install 目录复制，不在此表。
pub const OFFICECLI_DOMAIN_SKILLS: &[(&str, &str)] = &[
    ("pptx", "officecli-pptx"),
    ("word", "officecli-docx"),
    ("excel", "officecli-xlsx"),
    ("morph-ppt", "morph-ppt"),
    ("morph-ppt-3d", "morph-ppt-3d"),
    ("pitch-deck", "officecli-pitch-deck"),
    ("academic-paper", "officecli-academic-paper"),
    ("data-dashboard", "officecli-data-dashboard"),
    ("financial-model", "officecli-financial-model"),
    ("word-form", "officecli-word-form"),
];

/// OfficeCLI 退役兼容名单：共享技能目录可能仍留有这些 ID，不能作为普通技能放行。
pub const OFFICECLI_OFFICIAL_SKILL_IDS: &[&str] = &[
    "officecli",
    "officecli-pptx",
    "officecli-docx",
    "officecli-xlsx",
    "morph-ppt",
    "morph-ppt-3d",
    "officecli-pitch-deck",
    "officecli-academic-paper",
    "officecli-data-dashboard",
    "officecli-financial-model",
    "officecli-word-form",
];

pub fn catalog_plugin(id: &str) -> Option<&'static CatalogPlugin> {
    PLUGIN_CATALOG.iter().find(|p| p.id == id)
}

/// ego lite 安装补充：GUI 应用（macOS .dmg），Skill 由 Kivio 下载，无 MCP、无 brew。
const EGO_LITE_INSTALL_DOC: &str = r#"## 本插件补充（ego lite）

- **仅 macOS**。ego lite 是 GUI 应用（.dmg）；`ego-browser` 命令由 app 完成首次 onboarding 后注册到 PATH（通常 `~/.local/bin`）。
- **安装 app（二选一）**：
  1. 官网 / README 里的直链下载 .dmg：https://lite.ego.app/ ，下载后打开 .dmg 安装；
  2. 或运行技能自带脚本（若在本机）：`sh skills/ego-browser/scripts/install.sh`（macOS only）。
- 安装后**请用户在 app 内完成一次 onboarding**（可选导入 Chrome 数据），onboarding 会注册 `ego-browser` 到 `~/.local/bin`。等用户确认完成再继续。
- 验证：`command -v ego-browser`（找不到则 `export PATH="$HOME/.local/bin:$PATH"` 重试），再跑：
  ```
  ego-browser nodejs <<'EOF'
  console.log('ego-browser ready')
  EOF
  ```
- **Skill 由 Kivio 负责**：用户点「启用」后，Kivio 自动从仓库下载 `ego-browser` Skill 并接入对话——**你（AI）不要手写 Skill，也不要配置任何 MCP（本插件无 MCP）**。
- **无 brew cask**：不要 `brew install`。
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_plugins_keep_their_supported_install_paths() {
        assert!(catalog_plugin("officecli").is_none());
        let ego = catalog_plugin("ego-lite").unwrap();
        assert!(ego
            .install_commands
            .iter()
            .any(|c| c.platform == "macos"
                && c.command.contains("npx skills add citrolabs/ego-lite")));
        assert!(!ego.uses_shared_skill_dirs());
        #[cfg(windows)]
        assert_eq!(ego.host_install_command(), None);
    }
}
