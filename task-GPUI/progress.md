# 原生实施记录

2026-10-23，`codex/gpui-migration`。用户已选定 GPUI Kit，并授权根据实际结果调整推进顺序；TASK 作为范围与参考索引，不以完成文档阶段作为实现前置条件。Study 整个扩展仍在范围外。

当前状态以末尾“M1 用户否定后的视觉与交互返工”及收尾记录为准。前文保留实施历史；其中旧侧栏尺寸、早期未验证项和内部 ship 结论均有各自批次范围，不能代替本版状态或用户验收。

## 首批实现

独立 `native/` 工程，Kit 0.8.0 / `gpui-pre` 0.3.9。原生验证窗口使用内存合成会话，验证中文编辑、Markdown、流式输出、动态高度虚拟列表、停止与明暗主题；没有接入真实模型或用户数据。共享主题入口已补充到原工程规范。

原路径：React 输入/消息列表 → Tauri IPC → Rust 业务。此次仅实现原生展示验证路径：Kit Textarea → Preview 发送意图 → Transcript 离线输出 → MessageScroller。尚未替换任何业务路径或删除旧客户端。

## 基线与数据

- HEAD `2dafd0d95e7c482f3f6ff94c70e1dc4b294f8fcf`；本地 main `6840a2d5` 包含 CLI 修复。工作区原有外部 CLI/账号改动保留。
- [入口快照](baseline-inventory.json) 从当前工作区提取：16 个中心路由、17 个设置页签、359 个主命令注册，以及扩展 manifest。这是后续逐用例映射的索引，不等于完整功能验收。
- 正式身份 `com.zmair.kivio`，开发身份 `com.zmair.kivio.dev`。macOS 数据在 `~/Library/Application Support/<identifier>`；Windows 为 `%APPDATA%/<identifier>`（Roaming，无额外 `data` 层）。来源：`src-tauri/src/app_data.rs`。
- 用户目录 `.kivio` / `.kivio-dev`，OAuth service 为 `Kivio.ModelOAuth` / `Kivio.Dev.ModelOAuth`。验证窗口不调用这些路径解析或凭据入口，也未复制用户资料。
- 后续复用真实业务前，仍需明确原生实验身份、隔离数据副本和存储适配；不能让旧版与新版并发写同一份资料。

## 直接复用 Zeron

按用户最新要求，以 Zeron 为主要实现参考。`crates/markdown/src/{lib,parser,mend}.rs` 直接复制到 `native/vendor/zeron-markdown`，保留全部上游测试与根 MIT 许可证；`diff -qr` 确认 Rust 源码未改。来源 `zeronsh/zeron@c04487045e5f934818ab21bf5b7b7fbed16ef3d6`，依赖 pulldown-cmark 0.12.2。没有依赖被忽略的参考工作区。薄适配层仅把当前尾部的显示补全交给 Kit，原文和显示内容分开；半截链接的上游 pending 标记不会打开外部 URL。

目前不是 Zeron 整套渲染器：Kit 仍解析完整消息，尚有双重解析与整条文本复制，未达到单条超长回复按块虚拟化的目标。其输入和 transcript 耦合专用服务/协议与 GPUI fork，先保留 Kit 的成熟输入、焦点、滚动能力；后续逐模块适配，避免复制一套现有控件或进程负责人。

## 当前验证

- macOS Apple Silicon / Rust 1.99.0：`cargo build --locked` 通过。
- `cargo test --locked --workspace`：3 个 Kivio 测试 + 99 个原样保留的上游测试通过；覆盖真实输入订阅的 Enter/Shift+Enter/忙碌草稿、取消及迟到结果隔离、Markdown 适配的原文与代码块边界。
- `cargo clippy --locked --workspace --all-targets -- -D warnings`、`cargo fmt --check` 通过。依赖 `block 0.1.6` 有 Cargo future-incompatibility 提示，当前不阻塞构建。
- 激活的 macOS 运行依赖只有 gpui-pre 0.3.9，无 Tauri/Wry/WebView/GPUI Fast；锁文件包含未激活的可选包。
- 原生开发 bundle 身份 `com.zmair.kivio.gpui.preview`。CUA 实操确认中文粘贴、Emoji、多行发送、原文中的 `<tag>`/`&`/`**`、停止保留部分回复、10,000 条样本滚动与回到最新、浅/深主题；未调用真实模型。键盘集成测试先发现忙碌 Enter 导致换行，修复父级 action 消费后通过。
- 本机截图在 `.impeccable/review/gpui-{light,long-dark,stopped}-macos.png`（忽略目录，不进入提交）；只代表默认宽度 macOS 窗口。CUA 边缘拖拽没有实际改变尺寸，最小窗口布局未验证，不将同尺寸截图当作窄窗证据。
- Impeccable 本轮审查结论为通过：唯一要求修复的停止状态已显示“已停止”，并在原生截图中复核；此结论仅覆盖本轮发现及修复，不认证整个界面。对照 `DESIGN.md`、`.impeccable/design.json` 与原生源码，预览延续安静中性色、单一限宽阅读列和固定输入区；`theme.rs` 的部分文字/主色对比及 16px 基准字号属于原生局部适配，不升级为全局设计规则，原设计文档与 sidecar 保持不变。
- 尚未测量启动耗时、帧耗时、输入延迟、旧版性能对照；不以截图或单次 CPU/RSS 读数宣称性能达标。Windows、真实中文 IME 组合/候选框、屏幕阅读器、运行中减少动态效果变化、公式/Mermaid、终端、真实模型、安装升级均未验证。


## 下一步取决于验证结果

先解决输入/滚动/异步生命周期的实际问题，再抽离会话存储与设置读取的最小 Tauri 耦合。终端保持现有 PTY 生命周期负责人；不因参考组件自带终端后端就启动第二套进程。长文本性能分别测 UI 输入/帧耗时与模型/工具耗时，不以编译成功或 GPU 加速宣称性能倍数。

## 原生前端骨架（用户要求优先搭界面后）

已将技术验证窗口组织为可操作的前端：240px 可收起侧栏；会话创建、搜索、切换、重命名与移除；每条会话独立保留草稿、滚动位置和输出；空态提示填入草稿；顶部命令与快捷键；外观、快捷键、关于设置页。外观的主题、字号、减少动态效果即时生效，偏好仍只在本次窗口内。开发样本移入“更多”，不再占据聊天工具栏。窄窗默认收起侧栏；尚未做完整响应式实机验收。

参考 Zeron `crates/ui/src/shell.rs`、`shell/navigation_focus.rs`、`shell/sidebar_sections.rs`、`settings/appearance.rs`，控件使用实际发布的 Kit 0.8.0 `sidebar/`、`input/`、`switch.rs`、`menu/`。未照搬其服务协议或第二套输入实现；此前直接复用的 Markdown 模块继续使用。当前只有本次窗口的会话列表，没有引入项目/集的自动重排，既有 ADR 0004 不变。

本批按用户要求优先实现界面：仅格式化并完成 `cargo build --locked`，没有重复截图审查或扩大测试。上文的测试和审查结果属于此前的技术预览，不能视为这批整个前端的验收。模型、持久化及其余业务页面仍未接入。


## M1 原生核心前端交付（2026-10-23，新三阶段计划）

用户要求将整次重写重新规划为 M1 / M2 / M3，并完成 M1。任务文档第 7 节已替换旧阶段：M1 核心前端与本地交互；M2 业务接通、持久化及范围内功能补齐；M3 数据兼容、双平台、性能、打包升级和旧栈退出。以上是新的交付范围，不能把旧 M1 的完整双平台技术验证称作已完成。

### 已实现

- 以 Kivio `Sidebar.tsx` 为依据恢复最近 / 集 / 项目横排切换；保留插件、任务、作品入口位置，明确禁用并标识 M2 接入。新原生侧栏 256px，约 840px 以下默认折叠；按钮保留领域 ID。
- 会话创建、切换、搜索、重命名、归档/恢复和对话库；每个会话独立保留输入、滚动与生成实体。最近列表按活动序显示前 80 条，完整窗口内对话从库访问。
- 集/项目创建、名称编辑、显式上移/下移、项目目录原生选择器；会话分别关联一个集和一个项目。重命名/聊天不重排分组，新建置首；组内拖拽钉子属于 M2。
- 标题栏沿用 `ConversationTitlebarControls.tsx` 的运行时/模型入口；Kivio Agent / Chat 首条消息前可选，发送后锁定。模型只显示明确的离线示例，外部 CLI 尚未接入。
- 输入参考 Kivio `InputBar.tsx` 的自动增高和框外工具栏。原生文件选择器暂存附件，可移除；附件解析尚未接入，携附件时阻止发送且显示原因，不读取/上传附件。
- 基础设置保留即时主题、字号、减少动态效果；窗口重新激活不覆盖用户手动选择的减弱动画。快捷键与关于页可访问。
- Kit 默认资产并不包含所有 Lucide 名称：本次补齐仅需要的导航 SVG，未采用全量 AllAssets。复用 `public/logo-mark.png`，未生成替代品牌。

### 使用的技能与实现参考

使用 build-gpui-apps、gpui-kit、gpui-kit-design-guides、仓库 impeccable；新增两个跨会话/分组测试时使用 test-audit。工程规则继续使用原入口。

Kivio 产品依据：`src/chat/{Sidebar.tsx,InputBar.tsx,RuntimePicker.tsx,ConversationTitlebarControls.tsx}`、`src/settings/SettingsShell.tsx`、ADR 0004。Zeron `c0448704…` 的 `shell/sidebar_sections.rs`、`shell/navigation_focus.rs`、`composer.rs` 原生 picker 与任务生命周期、`transcript.rs` 尾部更新机制是实现参考；此前直接复制的 `crates/markdown` 继续使用，源码与 MIT 声明保留。没有复制 Zeron 产品导航、服务协议、GPUI fork 或浏览器实现。

Kit 参考 `examples/app_assets`、`website/docs/assets.md`、Sidebar/Textarea/Menu 示例；API 最终按实际 Cargo 源码 Component/Base/Kit 0.8.0、gpui-pre 0.3.9 核实。Kit Sidebar 仅接受 SidebarItem，Kivio 横排分区与稳定 ID 列表使用原生布局组合 Kit Button，未再造输入、菜单或焦点行为。

### 实际验证

- `cargo build --manifest-path native/Cargo.toml --locked` 通过；开发 bundle 已生成并在 macOS 原生窗口运行。
- `cargo test --manifest-path native/Cargo.toml --locked --workspace --quiet`：5 个 Kivio 测试与 99 个保留的 Zeron 测试全部通过。新增测试保护真实输入事件后的运行时绑定、跨会话 Entity/生成保留、项目改名不重排与新建会话归属。
- `cargo clippy --manifest-path native/Cargo.toml --locked --workspace --all-targets -- -D warnings` 通过。Cargo 仍报告上游 `block 0.1.6` future-incompatibility，未扩大到依赖升级。
- CUA 实际完成创建项目、项目下新会话、创建集、归属菜单、归档/恢复、Agent→Chat 选择、首消息后点击其他运行时仍保持锁定。检查中发现菜单误用不可点击 label，改为 Item constructor 后复验选择与锁定。
- 图标缺失与导航居中在原生截图中发现并修复；最终截图为 `.impeccable/review/m1-{home,chat,library,settings,narrow}-macos.png`。CUA 返回 JPEG 内容，沿用截图目录的 .png 文件名；未改图。桌面为 2320×1624 像素（1160pt 宽），实际缩窄为 1392×1624（696pt 宽），侧栏自动收起，输入和标题栏保持可见。
- 独立 Impeccable reviewer disposition **ship**：全部五张截图在 M1 原生离线核心前端范围内无实质阻塞；此结论不覆盖未测平台、全量业务或性能。

### 交付限制

所有会话、分组、目录选择、附件路径和偏好仅在当前窗口内；退出清除，不接触旧客户端资料或凭据。供应商、真实 Agent/Chat/CLI、工具权限、附件解析、持久化和其它页面属于 M2。完整中文 IME 组合态、屏幕阅读器、Windows、多屏/DPI、公式/Mermaid/终端、性能基准和正式安装升级属于 M3 未验证项，不因本次截图或编译标记为完成。深色主题此前技术预览做过原生检查，本次截图审查仅覆盖浅色。

文档收尾已对照 PRODUCT.md、DESIGN.md、`.impeccable/design.json`、`native/README.md`、TASK 第 7 节、原生主题/导航/输入/资源源码及上述五张截图。M1 延续 Kivio“安静的工作台”、中性表面、原品牌和导航关系；Kit 的原生控件尺寸、256px 侧栏与 16px 基准字号属于本次原生适配，不提升为全局设计规则。现有产品与设计文档及 sidecar 保持不变；其中 WebView/CSS 描述仍对应在用旧客户端，原生阶段边界由 TASK 与 native README 明确，未发现阻塞本次 M1 交付的文档矛盾。此收尾没有重跑测试或扩大审查；原生界面未运行只适用于 HTML/CSS 的 detector。


### 2026-10-23 — M1 用户否定后的视觉与交互返工

上一版 M1 的内部通过结论不代表用户验收。用户明确指出 UI 与交互太差、缺少实际参考，M1 重新打开。本轮直接运行原版 Kivio 与 Zeron 作对照：产品布局以 Kivio 为准；Zeron 用作成熟原生机制参考。

已改动：一体化 TitleBar、240px 可拖动侧栏（200–400px）、原版 Momo 静态形状与有限点击反馈、空态标题/输入区一组居中、框内发送、框外工具栏、用户消息右侧气泡与助手正文、消息复制；搜索浮层的方向键/Enter/Escape、分组展开收起与就地菜单、编辑/改名 Kit 弹窗。修正 Kit Dialog 默认 Enter 直接关闭导致未保存的问题，确认走 on_ok 并保留无效输入。

使用技能：build-gpui-apps、gpui-kit、gpui-kit-design-guides、仓库 impeccable、test-audit。实际 API：Kit/Component/Base 0.8.0、gpui-pre 0.3.9。主要依据：Kivio ChatConversationPane.tsx、Sidebar.tsx、chat-01-main.css、kivioBlobSim.ts；Zeron shell/sidebar_sections.rs、composer.rs、docs/performance-composer.md；保留原有直接复用的 Zeron Markdown。

返工第一轮验证：build 与 clippy 通过；6 项 Kivio 交互契约测试、99 项 Zeron 测试通过。真实 macOS 窗口已操作项目 Enter 保存、组内新建、组折叠、搜索/确认、流式回复、明暗主题和侧栏拖动。原版参考截图 m1-original-kivio.jpg；返工截图 m1-r2-{home,project,search,chat,settings-dark,chat-dark}-macos.jpg 位于忽略的 .impeccable/review/。这一轮的窄窗拖动受 UI 工具边缘命中限制，未把旧截图当成本版证据；后续收尾状态接下文。


返工收尾修正：原生最小窗口验证使用 debug-only `KIVIO_PREVIEW_COMPACT` 和 `bundle-preview.py --compact`，实际得到 680×520 逻辑像素。手动展开侧栏时发现保留的 Preview 仍缓存旧宽度；修正为随面板/窗口尺寸失效自身布局，聊天实体和草稿保持不变。侧栏范围同时约束主区至少 400px。当前已实际检查并记录默认收起、手动展开、深色设置、项目弹窗；未接入导航的 tooltip 实际可见，AX 同样包含解释；名称字段新增固定标签，深色标识复用原版 Momo。截图为 m1-r3-narrow-{home,expanded,settings-dark,project}-macos.jpg 和 m1-r3-unavailable-macos.jpg，位于忽略的 `.impeccable/review/`。

### 返工最终状态与证据边界

本轮视觉与交互返工已实现并完成所列验证。最终 build、fmt、clippy（`-D warnings`）通过，6 项 Kivio 测试通过；99 项未修改的 Zeron 测试已在本轮前序验证通过，收尾未重复运行。独立审查确认四项修复已解决，未见该修复批次引入可见回归；disposition 为 **ship，仅限 M1 离线预览范围**。这不是整个产品、全部平台或用户体验已验收的声明。

文档收尾对照原版截图、最终窄窗浅/深截图与原生源码：保留 Kivio 的中性表面、Momo 原形、侧栏入口关系和空态输入组合，采用 Kit 原生标题栏、面板与弹窗机制。当前侧栏为默认 240px、可调 200–400px，窄窗保留至少 400px 主区；历史 256px 记录已被本轮取代。Momo 只提供原版静态形状与 480ms 有限点击反馈，不表示完整桌面墨墨能力已接入。

`PRODUCT.md`、`DESIGN.md` 与 `.impeccable/design.json` 保持原样；设计文档的 WebView/CSS 控件说明仍描述在用旧客户端，原生阶段范围由 TASK 与 native README 记录，不在本轮改写全局视觉规范。模型、工具、持久化、插件与其他真实业务仍待 M2；Windows、完整中文 IME、屏幕阅读器、多屏/DPI、性能基准及安装升级仍未完成。Study 整个扩展继续排除。


### 2026-10-23 — 阶段性提交（暂停打磨）

用户要求先停止界面调整并提交当前成果。此记录覆盖上面的完成措辞：M1 仍待用户体验验收，最新交互没有完成原生复核，不继续启动视觉审查或打磨循环。

此前 r4 已调整 Kit 语义主题、原生分段导航、输入焦点反馈、消息/输入共同阅读宽度、窄窗设置与对话库布局；默认及 680×520 逻辑像素窗口已检查。独立审查的通过结论只覆盖 r4，不覆盖下列 r5 增强。

最新实现：输入 Up/Down 回看已发送提问并恢复草稿，Esc 停止生成，附件拖入及独立移除卡片；长用户消息折叠/展开，缓存有限摘要以减少折叠状态排版，复制始终保留原文；消息右键菜单与确认后回退编辑；侧栏分组拖拽排序、会话数量、组内新建与超过 20 条的查看全部入口。分组顺序仍遵守 ADR 0004，组内会话拖拽钉子尚未实现。

技能：build-gpui-apps、gpui-kit、gpui-kit-design-guides、impeccable、test-audit；提交说明使用 caveman-commit。产品行为依据 Kivio InputBar.tsx、hooks/useMessageActions.ts、MessageContextMenu.tsx、Sidebar.tsx；实现参考 Zeron c0448704 的 composer.rs、transcript.rs、shell/sidebar_sections.rs，以及 Kit list_story.rs。API 以实际 Kit/Component/Base 0.8.0、gpui-pre 0.3.9 为准。

本次提交检查：build、fmt --check、workspace clippy（-D warnings）通过；workspace 测试 7 项 Kivio + 99 项 Zeron 通过。上游 block 0.1.6 仍有 future-incompatibility 提示。最后一次原生检查确认长消息发送、折叠入口及历史回看；发现多行历史恢复后光标位于开头，已修正为末尾，并将多行/Emoji 加入历史与回退集成测试，该测试通过。修正后的原生窗口未复验；分组拖拽、外部文件拖放及本轮窄窗布局未完成实机确认。截图 m1-r5-messages-macos.jpg 记录修正前状态，仅作调查证据，不作为最终验收图。

当前仍是窗口内离线预览；业务、持久化和完整平台验收边界维持 M2/M3，Study 排除。参考仓库、target 构建产物与忽略的截图不进入提交。
