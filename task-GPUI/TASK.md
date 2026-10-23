# 用 GPUI 重写 Kivio：任务与验收

日期：2026-10-23（Asia/Shanghai）。状态：本轮 M1 视觉与交互返工已实现，并完成所列验证；上一版未获用户验收，本轮内部审查也不替代用户体验验收。M2 / M3 待实施。实际进度与未验证项见 [实施记录](progress.md)。

## 1. 目标与本次交付

使用 Rust + GPUI 重新实现 Kivio 桌面客户端的界面与平台层，替代现有 React / TypeScript / CSS + Tauri 实现。最终桌面界面不依赖 WebView。以现有产品能力、用户数据和行为契约为依据，按原生桌面的状态、布局与交互方式重新组织实现，按完整用例逐批交付。

重写不要求逐页翻译 JSX、照搬 DOM/CSS 结构或保留旧前端的调用层次。已有 Rust 业务逻辑按职责与耦合程度评估复用、解耦或重构；范围内的用户可见功能、数据兼容和既有 ADR 语义继续作为验收依据。重写也不自动授权删减功能、改变产品定位或全面替换视觉风格。

准备阶段已建立参考工作区；经用户授权，现在进入原生实现，按实际结果推进，不将本文件的阶段顺序当作前置审批。独立 `native/` 已接入 Kit 和 Zeron Markdown；未替换旧客户端、业务数据格式或发布流程。基线提交见 [references.lock.json](references.lock.json)；当前工作区另有外部 CLI 账号、检测、运行和设置界面的未提交改动，未提交改动不包含在该基线 SHA 中。后续实施必须重新检查并保留，不能 reset、覆盖或顺带提交。

- [x] 建立 `codex/gpui-migration` 分支。
- [x] 建立参考目录，克隆 GPUI Kit、Ely、Zed、Zeron 与 GPUI 技能源仓库。
- [x] 记录实际参考提交和检出范围。
- [x] 编写重写范围、阶段任务、兼容边界和验收条件。
- [x] 完成组件栈选型；跨平台完整验证纳入 M3。
- [x] 开始产品代码重写。

## 2. 已确定的方向与待决定的边界

已确定：GPUI 原生桌面 UI、移除 Tauri 与桌面 Web 前端、以 Rust 业务复用为主。首要目标平台沿用当前产品：macOS Apple Silicon 与 Windows x64；不因参考仓库支持 Linux 就扩张本次交付范围。

用户已明确选择 GPUI Kit，当前锁定发布包 0.8.0 / gpui-pre 0.3.9；用户随后指定 Zeron 为主要实现参考，优先直接复用适用模块，Ely 按需补充。已复制 Zeron 独立 Markdown crate 并保留 MIT 声明，不将其 GPUI fork 与 Kit 混用。跨平台完整验收仍未完成。

范围排除：Kivio Study 是独立扩展，整个扩展不属于当前 Kivio 主应用的 GPUI 重写范围，暂不做。包括文献库、阅读、文献翻译、批注、来源引用、Study 笔记与附件、导出、Word 插件及专用本地服务，均不列入开发、接入、技术验证、打包或完成验收。现有源码与用户数据暂留，不因排除范围而删除。后续需要时另行安排；主应用的扩展管理与接入机制仍在范围内，不以 Study 实现作为其前置条件。

以下其余边界在替换相应功能前形成明确决定，准备文档和独立调查无需等待：

| 边界 | 现状与候选处理 | 决策完成条件 |
| --- | --- | --- |
| 生成 HTML / 交互网页预览 | `src-tauri/src/html_preview.rs` 提供隔离来源，前端嵌入预览；普通 GPUI 文本控件不能执行任意网页 | 评估外部浏览器预览或原生受限展示，明确交互差异；不偷偷引入 WebView |
| 媒体与网络内容 | 网页播放器、媒体解码、远程图片、OAuth 打开浏览器不等于同一需求 | 分清应用 UI、媒体解码和外部浏览器；逐项记录替代能力和资源释放 |
| 发布身份与升级 | 现有正式身份 `com.zmair.kivio`，开发身份和存储/凭据分开 | 新版升级继续找到旧数据；实验运行使用隔离身份/数据副本 |

“全 Rust UI”不要求将系统 API 或 Swift OCR 辅助程序重写为 Rust。保留它们的现有业务用途，分别评估平台依赖。

## 3. 规则来源与重写时的冲突处理

工程规则唯一入口：[统一工程规范](../docs/engineering-standards.md)。产品范围见 [PRODUCT.md](../PRODUCT.md)，视觉依据见 [DESIGN.md](../DESIGN.md)，领域含义见 [CONTEXT.md](../CONTEXT.md)。本文件只记录重写任务，不复制一份平行规范。

现有规范要求复用 `Button.tsx`、共享 TS 控件、CSS token 与带类型的 Tauri Adapter；这些具体实现不能直接用于 GPUI。重写时保留“共享控件唯一归属、业务入口唯一归属、统一错误通知”的意图。在 M1/M2 启动原生实现前，在原规范中补充原生对应入口和旧栈适用范围，不把原生例外永久放在本文件。当前不修改仍在使用的 Web 规范。

既有 ADR 的行为必须在重写后保留：

| ADR | 必须保留的含义 |
| --- | --- |
| [0001](../docs/adr/0001-imported-cli-conversations-stay-on-their-cli.md) | 导入会话绑定原 CLI 与原工作目录 |
| [0002](../docs/adr/0002-imported-history-is-a-snapshot.md) | 导入历史是显示快照，不在每次打开时覆盖为 CLI 存储投影 |
| [0003](../docs/adr/0003-acp-agents-import-via-protocol.md) | ACP 枚举/加载依协议能力，不随 UI 重写改成解析私有存储 |
| [0004](../docs/adr/0004-sidebar-group-order-is-manual.md) | 集/项目手排，组内对话保留时间底座和钉子语义，最近列表时间序 |
| [0005](../docs/adr/0005-subagents-retain-identity-across-tasks.md) | 子代理身份和历史保留，执行资源独立，停止范围及恢复语义不变 |
| [0006](../docs/adr/0006-rewind-realigns-external-cli-sessions.md) | 回退外部 CLI 时同步处理原生会话，不能只裁剪界面历史 |

ADR 中历史 CLI 支持名单不是当前能力探测结果；保留行为不变量，能力仍由当前实现和实际协议决定。

## 4. 现有能力与重写范围

下列是当前负责区域，不表示其中所有 Rust 文件都已经与 Tauri 解耦。每个阶段先沿真实调用链核对状态与副作用归属。

| 能力 | 当前入口/目录 | 重写工作 |
| --- | --- | --- |
| 应用启动与组合 | `src/main.tsx`、`src/App.tsx`、`src/chat/Chat.tsx`、`src-tauri/src/lib.rs` | GPUI 启动、窗口与服务组合；保留退出清理语义 |
| 聊天与输入 | `src/chat/ComposerEditor.tsx`、`MessageList.tsx`、`ChatMarkdown.tsx`；`src-tauri/src/chat/` | 重写编辑、选区、虚拟滚动、富内容、附件；复用会话操作与存储 |
| 模型与 Agent | `src-tauri/src/api.rs`、`provider_request.rs`、`agents/`、`external_agents/`、`mcp/` | 保留模型协议和执行；替换 AppHandle、事件发送、窗口通知依赖 |
| 设置与身份 | `src/settings/`、`src/onboarding/`；`settings.rs`、`app_data.rs`、`provider_oauth/` | 原生表单/引导；保留默认值、规范化、账号、凭据和存储通道 |
| 扩展与市场 | `src/extensions/`、`src/chat/market/`、`extensions/sdk/`；`extensions.rs`、`plugins/`、`skills/`、`connectors/` | 替换 React 页面注册，保留启停、Hook、原生租约与权限归属 |
| Dock 与终端 | `src/chat/dock/`；`src-tauri/src/dock/` | 原生文件树、编辑/预览、终端；PTY 只保留一个生命周期负责人 |
| 屏幕工具 | `src/lens/`、`src/Lens.tsx`；`lens.rs`、`screenshot.rs`、`capture_geometry.rs`、`replace_translation/` | 原生框选、标注、翻译浮窗，保留截图/OCR/替换流程与权限 |
| 桌面集成 | `windows.rs`、`windows/`、`shortcuts.rs`、`desktop_pet/`、`lib.rs` | 窗口、托盘、快捷键、单实例、启动项、墨墨、焦点和多显示器适配 |
| 定时与自动化 | `src/chat/scheduledTasks/`、`automation/`；`scheduled_tasks/`、`automation/` | 重写 UI/节点编辑；保留调度、排队、绑定对话、取消与恢复 |
| 其他已有页面 | 笔记、知识库、搜索、备份、用量、媒体生成及连接器相关入口 | M0 登记完整菜单/路由清单，防止只完成聊天和设置便宣布整体完成 |
| 通信与打包 | `src/api/`、`src/generated/`、`src-tauri/build.rs`、`tauri.conf.json`、`scripts/`、`.github/workflows/` | Rust 直接调用/有生命周期的订阅；重新接回资源、安装和更新 |

## 5. 目标调用关系与状态归属

旧路径：用户操作 → React 页面/流程负责人 → typed Adapter → Tauri IPC → Rust 领域模块。

目标路径：用户操作 → GPUI Action/所属视图 → 现有领域操作 → Rust 结果或订阅事件 → 所属 Entity 更新。

业务中真实的进程、网络和外部 CLI 协议仍然存在；移除的是桌面 UI 与同进程 Rust 业务之间的 Tauri IPC。不为替换 IPC 新建通用 RPC 框架。

- 会话持久化、模型执行、任务调度等权威规则仍由原领域负责人持有；目前位于 TS 的业务规则必须识别并在 Rust 中重新落实，不能一概当作展示代码丢掉。
- 输入草稿、焦点、选区、滚动和局部展开状态由对应 UI 生命周期持有；保存草稿的持久化规则仍走领域入口。
- Tokio / 子进程工作不阻塞 GPUI UI 线程；有返回值的操作用直接结果，流式过程用有明确生命周期的订阅，保留取消、终态与迟到结果隔离。
- UI 更新可合并，但不能丢失工具完成、权限请求等业务事件；按实际负载确定背压与缓冲策略。窗口卸载只释放其订阅，不擅自取消领域持有的后台任务。
- 移除 Tauri IPC 不代表取消权限与准入：工具授权、扩展启用/租约、文件路径与外部链接检查继续由原负责人执行。
- 先抽出真实复用的领域边界，再按需要分 crate；不预建全仓 Service/Manager/Controller 层。
- 旧客户端作为开发期对照可暂留；新客户端内部不嵌旧页面。两者复用同一业务实现，禁止同时写同一份真实数据。

## 6. 参考使用方法

执行时必须遵循根目录 [AGENTS.md](../AGENTS.md) 的 GPUI 技能与参考查阅要求。每个实现任务先使用相关技能，再查对应组件或机制的参考源码；在该任务的完成记录中简要列出技能、参考路径或版本和验证结果。参考资料用于实际实现与核验，不只是准备阶段收藏。

参考版本统一读取 [清单](references.lock.json)，不要把上游当前 main 与旧技能示例混合使用。

| 参考 | 重点读取位置 | 限制 |
| --- | --- | --- |
| GPUI Kit | `crates/kit/`、`crates/component/`、`crates/base/`、`skills/`、`website/docs/` | 作为组件栈候选；核对版本、features 和源码签名，不把全部可选能力引入应用 |
| Ely | `crates/ely-chat/`、`ely-agent/`、`ely-documents/`、`ely-terminal/`、`ely-core/`、`compat/kit/` | 当前顶层包 macOS 依赖 Wry；优先核查子包依赖闭包，不整包照搬 |
| Zed | `crates/gpui/`、`crates/gpui_platform/`、`crates/ui/` | 稀疏检出仅用于阅读；不是独立可构建的完整 Zed 工作区，不复制整个编辑器架构 |
| Zeron | `crates/ui/src/{composer.rs,transcript.rs,markdown/,terminal/,shell/}`、`crates/harness/`、`crates/client/`、`crates/theme/` | 完整 Agent 应用参考；自维护 GPUI fork 与内嵌浏览器依赖不能直接引入，保留 Kivio 的领域与 ADR 语义 |
| GPUI 技能 | `skills/build-gpui-apps/` | 参考版本、输入法、焦点、平台、验收方法；不复制其项目特有要求 |

Ely 的文档模块可参考 Rust Markdown、公式与 Mermaid 实现；其 `DocumentViewer` 接受宿主提供的页面图像/区域，不等于完整 PDF 引擎。终端候选可能自带 PTY，接入时避免与 Kivio 现有 PTY 重复启动或重复拥有进程。

Zeron 的优先参考点：M1 的输入框、长消息、Markdown 与原生外壳；M2 的外部 CLI、终端与工作台机制；M3 的性能与资源生命周期。入口见上表，实际 API 以采用的依赖版本核实。当前参考快照的 `Cargo.toml` 使用 `zeronsh/zui` 与 `zeronsh/gpui-component` fork；`crates/ui/Cargo.toml` 含 macOS Wry/WKWebView 和 Windows WebView2 依赖，因此不能把整个 UI crate 当作无 WebView 的组件库采用。其 daemon/RPC、CRDT 与多设备同步服务于自身产品，不自动成为 Kivio 的架构目标，也不覆盖现有退出与会话持久化语义。根许可证为 MIT，摘取时仍核对模块及第三方声明。Zeron 整体应用未构建；直接复用的 Markdown 模块及本地验证见实施记录。

依赖和摘取源码前记录各模块实际许可证与资源许可证，保留必要声明；Zed 仓库不同模块的许可分别检查，不假设全仓都与 GPUI 相同。

## 7. 三阶段交付（2026-10-23 重新划分）

按用户最新要求，将原 M0–M6 合并为 M1 / M2 / M3。下面是新的验收边界，旧编号不再用于判断完成。产品与视觉依据是 **Kivio 原前端**；Zeron、Ely、Kit 提供组件和实现机制，不替换 Kivio 的信息架构。Study 全部排除。

### M1 — 原生核心前端与本地交互

目标：可运行、可操作的 Kivio 核心前端，先把用户每天接触的主结构搭好。仅使用隔离的窗口内数据和明确标识的离线回复；不宣称已接通业务或持久化。

- [x] 最近 / 集 / 项目横排导航、对话库及搜索、空状态、侧栏收起与窄窗适配。
- [x] 会话新建、切换、独立草稿、重命名、归档/恢复；集与项目创建、编辑、归属和手动顺序，项目目录选择。
- [x] 标题栏 Kivio Agent / Kivio Chat 选择；首条消息后锁定运行时；模型与 CLI 接入状态如实可见。
- [x] 原生输入区：自动增高、发送/停止、附件选择与移除、框外工具栏；未接入能力明确禁用说明。
- [x] 原生消息阅读、流式 Markdown、回到最新、切换会话不重建视图；取消隔离迟到内容。
- [x] 基础设置（外观、字号、减少动态效果、快捷键、关于）；保留原应用导航关系，未接业务入口禁用。
- [x] 按影响完成编译、交互契约测试与 macOS 原生窗口检查，记录实际结果与限制。

验收：以上本地交互可连续操作，入口和状态对应真实实现。M1 不是完整产品替换；Windows、中文输入法组合态、性能基准和全量富内容验收属于 M3，不把未测项写成通过。

### M2 — 业务接通与范围内功能补齐

- 从真实用例解耦已有 Rust 业务的 Tauri 依赖；原生视图直接调用领域入口，业务核心不依赖 GPUI。
- 接回会话/项目/集/草稿持久化、完整历史搜索、组内拖拽钉子、导入、回退、分支、重试与恢复；保留全部 ADR。
- 接入供应商、模型、凭据/OAuth、内置 Agent/Chat、外部 CLI/ACP、工具与权限、子代理，以及附件解析/引用。
- 补齐全部设置、插件/技能/MCP/连接器、笔记/知识库、搜索、用量、备份、媒体生成；完整功能清单逐项落实。
- 重写 Dock 文件树/编辑/差异/终端、定时任务、自动化；保留进程与后台任务唯一生命周期负责人。
- 接入托盘、单实例、窗口恢复、全局快捷键、启动项、剪贴板、Lens/截图/OCR/翻译、墨墨与浮窗。
- 主应用扩展改为原生注册，保留启停、权限、保存屏障与租约；Study 不开发、不接入。
- 明确 HTML/媒体预览的无 WebView 替代能力及用户可见差异。

验收：功能清单内用例在真实领域实现上跑通成功、失败、取消、离开、重开恢复；CLI 历史、数据与工作目录不变义，不以演示结果或空页面替代。

### M3 — 兼容、性能与正式替换

- 真实旧数据副本升级、备份恢复、异常中断、多窗口/重复启动验证；正式身份、数据目录与凭据连续。
- macOS Apple Silicon / Windows x64 实机完成 IME、选区、撤销、焦点、多屏/DPI、权限、终端 TUI、富内容与资源释放验收。
- 同一数据与硬件测量启动、输入延迟、P95 帧耗时、CPU/内存、流式阅读及长列表；依据结果优化，不以库宣传代替测量。
- 替换打包、签名、安装、更新、资源、CI 与发布检查；正式产物仍由 GitHub Actions 产生。
- 功能对照完成后退出 React/Tauri 桌面入口及无调用依赖；保留范围外 Study 的源码和资料，不顺带破坏共享依赖。
- 更新 README、开发命令、工程规范与发布文档；目标客户端依赖闭包无 Tauri/Wry/WebView。

验收：完整原生客户端可安装、升级、恢复；范围内功能逐项有证据，任何降级需明确记录，不能通过删除入口完成验收。

## 8. 验证与证据

当前旧版的检查入口来自 `package.json` 与 [发布规范](../docs/RELEASE_PACKAGING.md)：

```sh
npm run lint
npm run typecheck
npm run architecture:check
npm test
cargo test --manifest-path src-tauri/Cargo.toml
npm run build:ui
npm run package:check
```

以上不是本次已执行结果，也不要求每个文档改动运行全套。重写每一批按影响选择现有检查；旧入口删除时同步替换其检查，不能提前关闭。新增原生 workspace 后提供真实可运行的 fmt/check/clippy/测试命令，锁文件构建使用 `--locked`；此时不编造尚不存在的 crate 命令。编写或修改测试时使用 `test-audit` 技能，验证业务结果与交互契约。

| 场景 | 证据 |
| --- | --- |
| 中文输入与编辑 | macOS/Windows 真实输入法；组合输入不误发送，候选框跟随，撤销/选择/粘贴正确 |
| 长对话与流式输出 | 同一脱敏数据集、窗口尺寸、缩放、硬件与构建模式；输入延迟、P95 帧时间、总进程内存、CPU |
| 焦点/主题/缩放 | 键盘完成核心操作；Escape 关闭最上层并恢复焦点；明暗主题、字体缩放、减弱动画 |
| 异步与恢复 | 快速换会话、关闭窗口、取消后迟到事件、重开恢复，状态不会串到新会话 |
| 安装升级 | CI 产物、解包文件和版本、升级前后数据、失败恢复；签名与未签名状态如实记录 |

性能指标先测旧版再确定重写目标；GPU 加速、编译通过或库宣传不能代替测量。模型首 token、工具执行时间与 UI 输入/绘制延迟分开报告。

## 9. 实施状态

2026-10-23：按用户要求暂停 UI 打磨，提交 M1 当前阶段成果；最新输入、消息与侧栏交互的原生复核尚未完成，M1 未获用户体验验收。详见实施记录最后一节。

以 [实施记录](progress.md) 的实际完成项、验证证据和未验证项为准。实现入口为独立 `native/`；参考目录不直接加入 Cargo workspace，摘取的第三方源码记录版本与许可。
