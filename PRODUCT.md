# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

此字段表示 React WebView 界面的设计平台，不表示提供浏览器版产品。Kivio Desktop 是面向 macOS（Apple Silicon）与 Windows 10/11 的 Tauri 2 桌面应用；现有代码使用 React、TypeScript、Vite 与 Rust。

## Users

开发者与技术用户、知识工作者同等重要，不将任一群体的主要场景降为附属功能。

- 开发者与技术用户：在项目和工作目录中使用 Agent、工具、子代理及本机外部 CLI 代理完成工作。
- 知识工作者：阅读、研究、翻译、理解屏幕内容及处理资料。

## Product Purpose

以统一 Agent 工作台为核心，在同一聊天界面承载内置运行时与用户本机已安装的外部 CLI 代理；同时通过常驻托盘、热键、翻译、截图 OCR 与 Lens 连接桌面上的工作内容。

产品成功意味着用户可以在一致的客户端中使用所选运行时完成任务，并从屏幕级工具进入相关对话，不必将这些工作拆成互不相连的工具。

## Positioning

统一 Agent 工作台是用户确认的首要产品定位。Kivio 将完整的内置 Agent、只读研究运行时 Kivio Chat 与本机外部 CLI 代理纳入同一聊天界面，并结合屏幕级翻译、OCR 和视觉问答。该描述是现有机制，不构成市场唯一性、性能领先或效果保证。

## Operating Context

- 应用常驻托盘，通过全局热键打开聊天、翻译或 Lens；客户端也提供完整的项目与对话工作界面。
- 首次使用完成供应商与热键引导；macOS 屏幕相关功能需要相应的辅助功能与屏幕录制权限。
- 用户选择并配置模型供应商；已有外部 CLI 代理的可用性取决于本机安装情况。
- Lens 冻结屏幕后支持框选提问与标注，并可将对话交回客户端。
- 项目带根目录；原生会话续聊保持原 CLI 与原工作目录的绑定。
- 开发入口：`npm run dev` 启动桌面应用，`npm run dev:ui` 在端口 5713 启动 Vite 前端。浏览器前端预览不能代替系统权限、原生窗口、托盘、热键与 CLI 集成的桌面验证。

## Capabilities and Constraints

- 聊天支持工具循环、子代理、Skills、MCP、知识库、附件与多模型回答；内置运行时与外部 CLI 代理具有不同的能力与历史所有权，不将它们视为可任意互换的后端。
- 屏幕工具包含快速翻译、选中翻译、截图 OCR 翻译、替换翻译与 Lens；OCR 使用系统引擎或可选的 RapidOCR 离线包。
- 客户端还包含笔记、定时任务、自动化、媒体生成、插件及连接器等已有能力。
- 无需 Kivio 账号；设置、对话、知识库与笔记保存在本机，无遥测。模型请求发往用户配置的供应商；本机保存不等于模型处理全程离线。
- 供应商可按用途与对话配置；API Key 可留空时直接请求所配置地址，不伪造认证成功；OAuth 仍需登录。
- 数据所有权与领域命名以 `CONTEXT.md` 为准：使用“外部 CLI 代理”“内置运行时”“原生会话”“会话绑定”“工作目录”“集”“项目”等既有术语。导入用于显示历史并建立绑定，不转移原生会话的所有权；续聊由原 CLI 承担。
- 定时任务向绑定对话发送消息；自动化运行独立的节点工作流，两者不是同一能力。
- 工程约束继续由 `docs/engineering-standards.md` 及适用的领域 ADR 维护；此档案不建立平行工程规范。

## Brand Commitments

- 保留现有名称 Kivio / Kivio Desktop 与真实产品术语。
- 已有品牌资产：`public/icon.png`、`public/logo-mark.png`。
- 本次初始化不决定或替换视觉风格、配色、字体、组件与页面概念；既有界面仍是后续局部改进的视觉依据。

## Evidence on Hand

- `README.md`、`README.en.md`：产品介绍、功能、系统要求、安装与使用说明。
- `CONTEXT.md`：领域术语及会话、项目、定时任务与自动化的语义。
- `docs/screenshots/chat-client.png`、`docs/screenshots/settings.png`、`docs/screenshots/screenshot-translation.png`、`docs/screenshots/lens-formula-extraction.gif`：现有产品展示素材；不假定这些素材与当前界面逐像素一致。
- `docs/releases/v3.1.1.md`：已有版本记录。
- `LICENSE`：现有 GPL-3.0-or-later 许可依据。
- 本次未确认客户评价、量化效率提升、竞争排名或性能基准；后续设计不得将其编造成证明材料。

## Product Principles

1. 统一工作台优先：不同运行时共享一致的聊天体验，同时明确其真实能力、限制与历史所有权。
2. 两类用户同等重要：屏幕级知识工作与开发者 Agent 工作均保留完整的主要流程。
3. 用户掌控供应商与本机数据：不引入未经确认的账号依赖、遥测或中转承诺。
4. 尊重现有工作连续性：项目、工作目录、会话绑定与正在执行的任务遵循既有语义，不以界面简化掩盖数据或状态变化。

## Open Decisions

- 尚未确认产品级无障碍合规标准或额外的用户群体需求；不宣称已满足某个认证标准。
- 尚未确认量化产品成功指标或特定竞品对比结论。
