# 中文

Kivio v2.7.5 是一次功能大版本：多模型并行回答、上下文压缩系统重构、全新首启引导，以及 Gemini 原生接入。

- **多模型并行回答**：一次提问可并排对比多个模型的回答，各回答独立并发生成、互不阻塞；某个模型报错时保留为独立错误列，不影响其他答案，可点选任一列作为进入后续上下文的答案。
- **上下文压缩重构**：统一压缩路径并强化摘要质量；新增 microcompact 微压缩层（动用 LLM 摘要前先做轻量压缩）、压缩时间线 UI 与多次压缩后的准确度衰减提示；压缩分隔线锚定在触发压缩的那条消息。
- **首次引导**：全新分步引导（供应商 → 网络搜索 → 快捷键 → 完成），按系统语言自动本地化、内嵌 API Key 申请链接；加固了重启、加载错误与供应商校验。
- **模型接入**：新增 Google Gemini 原生 `generateContent` 适配器，与 OpenAI / Anthropic 平级，走原生协议绕开 OpenAI 兼容端点对未知字段的 400 严格校验。
- **网络搜索**：全新两栏设置面板，接入 Exa MCP / Ollama / Grok，可面板内测试搜索，结果卡片显示服务名与真实 logo。
- **Lens**：新增替换翻译，用 RapidOCR 定位并在原文字位置以画布覆盖译文；修复选中翻译浮窗偶发黑框。
- **Chat**：可编辑已发送消息并从该处重新生成；新增 Diagram 技能与 Mermaid 流式渲染；对话显示所属项目；工具调用折叠行改为 Cursor 风格。
- **开发**：新增应用内请求调试面板（密钥自动掩码、可复制为 cURL）。
- **修复**：发版前完成分主题代码审查，核实后修复 10 项正确性问题（严格端点缓存键 400 风险、失败臂回灌上下文、引导保存失败无提示 / 完成死路 / 语言覆盖、压缩衰减提示被抹除、交付目录吞产物等）。
- **发布**：Windows NSIS `.exe` 本地构建并上传；macOS Apple Silicon DMG 由 GitHub Actions 自动构建。

# English

Kivio v2.7.5 is a feature release: multi-model parallel answers, a reworked context-compaction system, a brand-new first-run wizard, and native Gemini support.

- **Multi-model parallel answers**: ask once and compare several models side by side — each answer streams concurrently and independently; a failed model becomes its own error column without aborting the others, and you can pick any column as the answer carried into later context.
- **Context compaction rework**: unified compaction paths with hardened summary quality; new microcompact tier (a light pass before the LLM summary), a compaction timeline UI, and an accuracy-decay hint after repeated compaction; the divider is anchored at the message that triggered compaction.
- **First-run wizard**: a new step-by-step setup (provider → web search → hotkeys → done), auto-localized by system language with inline "get API key" links; hardened restart, load-error, and provider validation.
- **Model support**: new native Google Gemini `generateContent` adapter, a peer of OpenAI / Anthropic, using the native protocol to avoid the strict 400-on-unknown-field behavior of Gemini's OpenAI-compatible endpoint.
- **Web search**: a new two-pane settings panel with Exa MCP / Ollama / Grok, in-panel search testing, and result cards showing the service name and real logo.
- **Lens**: new replace-translation that locates text with RapidOCR and overlays the translation in place on a canvas; fixed an occasional black frame on the selected-translation overlay.
- **Chat**: edit a sent message and regenerate from it; new Diagram skill with robust Mermaid streaming render; conversations show their project; tool-call rows switched to a Cursor-style layout.
- **Dev**: new in-app request debug panel (secrets auto-masked, copy-as-cURL).
- **Fixes**: a pre-release code review across subsystems, then 10 verified correctness fixes (strict-endpoint cache-key 400 risk, errored arm leaking into context, onboarding save-failure with no feedback / finish dead end / language override, compaction decay warning being wiped, delivery-dir eviction eating a fresh artifact, and more).
- **Release**: Windows NSIS `.exe` built locally and uploaded; macOS Apple Silicon DMG built by GitHub Actions.

---

**安装 / Install**

- **macOS**：DMG 未签名，首次打开请右键 → 打开，或运行 `xattr -cr /Applications/Kivio.app`。
- **macOS**: the DMG is unsigned — on first launch, right-click → Open, or run `xattr -cr /Applications/Kivio.app`.
