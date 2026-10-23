# Kivio 原生客户端

当前为 M1 原生核心前端的阶段性保存版本。最新输入、消息与侧栏增强已实现并通过代码检查，但原生交互复核尚未完成；用户要求暂停打磨并先提交，不代表 M1 用户验收通过。以 Kivio 原有前端为产品依据，Zeron 为实现参考。M2 接通业务与补齐功能，M3 完成兼容、性能与正式替换。阶段验收见 [三阶段计划](../task-GPUI/TASK.md#7-三阶段交付2026-10-23-重新划分)。尚未接入模型、工具、真实会话存储或系统集成；Study 整个扩展不在本次重写范围内。

```sh
cargo run --manifest-path native/Cargo.toml --locked
cargo test --manifest-path native/Cargo.toml --locked --workspace
cargo clippy --manifest-path native/Cargo.toml --locked --workspace --all-targets -- -D warnings
cargo fmt --manifest-path native/Cargo.toml --check
```

从仓库根目录执行。工具链最低声明 Rust 1.95，当前验证使用 Rust 1.99.0；macOS 构建需要 Xcode Command Line Tools。Kit 使用发布版本 `=0.8.0`，实际依赖由本目录 `Cargo.lock` 锁定；不依赖 `task-GPUI/references/` 路径。启用默认组件及按需嵌入的导航图标，`profiler` 为显式可选，未启用 GPUI Fast、JS Shell 或 WebView。锁文件会包含可选依赖，实际激活闭包用 `cargo tree` 判断。

macOS 可先构建，再执行 `python3 native/scripts/bundle-preview.py` 生成 `native/target/Kivio Native.app`，供本地原生交互检查；该脚本不负责签名或发布。

侧栏沿用 Kivio 的“最近 / 集 / 项目”横排切换和插件、任务、作品、扩展的入口关系；这些业务入口在 M1 禁用，接入属于 M2。侧栏默认 240px，可在 200–400px 间拖动（窄窗同时保留至少 400px 主区）；收起不使用宽度动画。搜索使用 Kit 弹窗，支持上下键、Enter 和 Escape；分组支持展开/收起，组与对话都有就地操作菜单；编辑使用 Kit 弹窗保留当前页面。支持新建、搜索、切换、重命名、归档/恢复和移除预览对话。集与项目可创建、改名、上移/下移，项目使用原生目录选择器记录工作目录；对话可分别归属一个集和一个项目。分组按显式顺序排列，新建放首位，聊天活动不重排分组；组内拖拽钉子与持久化留在 M2。最近列表取活动顺序的前 80 条，完整窗口内会话从对话库访问。

每条对话保留自己的输入草稿、滚动位置与输出任务，切换不会重建。标题栏 Kivio Agent / Kivio Chat 可在首条消息前切换，之后锁定；模型菜单明确显示离线示例，外部 CLI 不冒充已连接。空态按原版将 Momo、标题和输入区居中组合；有消息后输入区贴底。发送按钮在框内，工具栏位于框外，输入框自动增高；Enter 发送、Shift+Enter 换行，停止保留部分回复；用户气泡靠右，助手正文靠左，消息可复制。附件可以通过原生文件对话框暂存/移除，未接解析服务时阻止携附件发送，并提供原因；不会读取或上传附件。

设置包含实际生效的明暗/系统主题、字号、减少动态效果、快捷键和关于页。系统重新激活不覆盖本窗口手动选择的减少动态效果偏好。短对话、1,000 条与 10,000 条固定样本在对话操作菜单中，每次创建独立示例。数据仅在内存中，关闭即清除。该窗口不读取或写入 `com.zmair.kivio`、`com.zmair.kivio.dev`、`.kivio`、`.kivio-dev` 和任何 OAuth 凭据。

## 归属与性能

- `main.rs`：应用启动、窗口、退出。
- `shell.rs`：会话视图生命周期、窗口内状态、搜索、重命名与键盘 Action；`shell/navigation.rs` 侧栏，`header.rs` 运行时及归属菜单，`library.rs` 对话库和分组编辑，`settings.rs` 基础设置。窗口关闭释放全部对话和任务。
- `assets.rs`：组合 Kit 默认资源与所需的少量图标，嵌入仓库已有 `public/logo-mark.png`；不把全量图标目录加入二进制。
- `preview.rs`：单条会话的空态、输入和发送意图；输入实体持续存在，切换会话和忙碌时保留草稿。
- `transcript.rs`：合成数据、当前输出任务、消息列表。只对变化的尾部消息请求重新测量；消息体用 `SharedString`，滚动回调不复制整个历史。列表虚拟化、滚动锚点与跳转按钮复用 Kit。
- `streaming_markdown.rs`：将直接复用的 Zeron 增量解析和尾部补全接到 Kit 文本控件；仅当前生成持有解析器，显示补全不写入原文，完成/停止后恢复原文。
- `theme.rs`：Kivio 主题映射，控件沿用 Kit 的语义尺寸、焦点和交互。

分组编辑使用保留的原生输入 Entity；文件选择异步返回且任务与所属编辑器/对话绑定。侧栏列表使用领域 ID，避免重排导致焦点错位；Kit Sidebar 0.8.0 仅接受 SidebarItem，Kivio 的横排分区和自定义列表用原生布局组合 Kit Button，未复制控件交互实现。

动效使用 Kit 的有限过渡：流式文字渐显、回到最新按钮显隐；Momo 点击触发 480ms 单次轻跳，遵守减少动态效果，不启动常驻计时器。没有持续背景动画或自建动画循环。Kit 初始化及窗口重新激活时读取系统减少动态效果设置。当前只做消息级虚拟化：Zeron 负责增量识别尾部边界，Kit 仍接收整条显示文本并自行解析，因此还存在双重解析和整条文本复制；不能宣称已获得 Zeron 的块级渲染性能。长回复按块虚拟化、解析调度和完整性能采样仍需后续完成。用户消息通过 Kit 的 HTML 文本解析器转义显示并保留换行，该控件没有浏览器或 WebView。

## 参考与约束

使用 `build-gpui-apps`、`gpui-kit`、`gpui-kit-design-guides`、仓库 `impeccable` 与 `test-audit`。规则唯一入口仍是 [工程规范](../docs/engineering-standards.md)，不在此另建规范。

参考版本见 [清单](../task-GPUI/references.lock.json)：

- **Kivio 产品依据** `src/chat/{Sidebar.tsx,ChatConversationPane.tsx,ConversationTitlebarControls.tsx,InputBar.tsx,RuntimePicker.tsx,kivioBlobSim.ts}`、`src/settings/SettingsShell.tsx` 和 ADR 0004：导航、运行时锁定、框外工具栏与手动分组顺序。
- GPUI Kit `crates/kit/src/lib.rs`、`crates/component/src/message_scroller.rs`、`crates/story/src/stories/message_scroller_story.rs`、`website/component/textarea.md`：启动、列表和输入。
- Ely `crates/ely-chat/src/list.rs`：阅读位置与新消息反馈。
- **主要实现参考 Zeron** `crates/markdown/`：直接复制独立解析模块及其测试，位于 `vendor/zeron-markdown`；上游 `c04487045e5f934818ab21bf5b7b7fbed16ef3d6`，Rust 源码未修改。`crates/ui/src/shell.rs`、`shell/navigation_focus.rs`、`shell/sidebar_sections.rs`、`settings/appearance.rs` 用于前端结构、焦点和外观参考；`crates/ui/src/transcript.rs`、`composer.rs` 用于核对块级虚拟化、流式滚动与输入边界；没有引入其 fork、浏览器、RPC。接下来优先评估它的块级渲染、终端与进程生命周期，适配 Kit 和现有 Kivio 业务的必要边界。
- 最终 API 核对使用 Cargo 下载的 Kit/Component/Base 0.8.0 与 `gpui-pre` 0.3.9 源码。参考快照与发布包存在差异，以实际发布包为准。

Zeron 复制部分保留 [MIT 许可证](vendor/zeron-markdown/LICENSE) 与 [来源记录](vendor/zeron-markdown/UPSTREAM.md)；只将 Cargo 的 workspace 继承字段改为独立配置。Kit/Component/Base 为 Apache-2.0；GPUI 子包许可由实际 Cargo 元数据记录，正式打包前仍需完整第三方许可证与资源清单。

验证记录见 [原生实施记录](../task-GPUI/progress.md)。

Momo 的静态形状直接来自原版 `BLOB_REST`，不是重新绘制。运行 `node native/scripts/export-momo.mjs` 可重新导出 SVG；Node/esbuild 仅用于开发时导出，不属于原生运行依赖。标题栏、可缩放面板、对话框及输入控件使用实际发布的 Kit/Component/Base 0.8.0，参考 Zeron `shell/sidebar_sections.rs` 与 `docs/performance-composer.md` 的折叠、局部更新及有限动效原则。

最小窗口验证：构建后执行 `python3 native/scripts/bundle-preview.py --compact` 生成独立的 `Kivio Native Compact.app`，以 680×520 逻辑像素启动。该尺寸预设仅在 debug 构建生效；正式构建忽略 `KIVIO_PREVIEW_COMPACT`。
