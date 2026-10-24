# Kivio GPUI 客户端

当前启动入口使用 **完整移入的 Zeron UI 源码**，在 `zeron-ui/` 内适配 GPUI Kit 0.8.0。`src/main.rs` 直接启动它；旧 `src/shell.rs`、`src/preview.rs` 保留供迁移时提取业务接线，已不参与主程序编译。不是基于旧预览重新模仿 Zeron 的布局。

```sh
npm run dev
# 或
cargo run --manifest-path native/Cargo.toml --locked -p kivio-native
cargo build --manifest-path native/Cargo.toml --locked -p kivio-native
python3 native/scripts/bundle-preview.py
cargo test --manifest-path native/Cargo.toml --locked -p kivio-native --test kivio_port
```

macOS 开发包生成于 `native/target/Kivio Native.app`。退出当前应用后再启动新包。`npm run build` 构建原生 release 二进制；旧 Tauri 开发/构建命令保留为 `dev:tauri-legacy`、`build:tauri-legacy`，用于尚未迁出的源码参考，不是 GPUI 启动路径。此包尚非签名发布安装包。

## 已接通的路径

- Zeron 原版 Shell、侧栏、输入框、消息与 Markdown 视图、设置等源码直接作为应用界面。
- 对话列表、历史消息、项目读取以及对话改名/归档由 Kivio 的 `ConversationStore` 和 `group_catalog` 提供。
- 文本发送使用已有 `src/chat_runtime.rs` → `kivio-model-runtime`，保留供应商适配器及规范消息；停止、流式显示、保存、重开历史已接入。
- 输入框的「模型 / 供应商」打开已有 Kivio 供应商编辑器。新对话使用已保存的默认模型，已有对话继续使用其保存的供应商和模型。供应商编辑器接入设置 → 模型，模型分工移到混音器。主窗口使用 Kit Root，支持下拉框、通知与焦点管理。尚未恢复原版 Zeron 的行内模型选择交互。
- 设置侧栏按 Kivio `SettingsShell.tsx` 的 17 个分类替换 Zeron 导航，复用 Zeron `settings/widgets.rs` 的页面列与滚动框架。基础、主题、模型、快捷键、翻译、Lens 等使用 Kivio 字段；未接通功能禁用或显示只读状态，不能视为全量设置迁移完成。
- 设置迁移以安装版 Kivio 与 `src/settings/` 原代码共同为依据。模型恢复供应商列表/详情布局；翻译、Lens、Agent 按原分组展示提示词，语言与提示词通过原设置字段保存。
- 主题页读取两端共用的 `src/theme/builtins.json`，包含原版 12 套明暗色板，应用时同步到 Zeron 与 Kit 控件。自定义主题编辑、导入导出尚未迁入。
- 对话库复用 Rust 检索/排序规则，支持五个范围、全文检索、分页、打开与归档/恢复。L1/L2 记忆编辑复用原文件负责人、配额及内容校验；自动记忆注入仍未接通。
- 使用统计读取原调用账本，显示汇总、趋势和调用明细。Hooks 提供事件、命令/HTTP 编辑和冲突检查；事件派发仍未接入。网络搜索恢复 14 个服务商的详情与默认选项，保存只变更编辑过的字段；TinyFish OAuth 授权入口仍未接入。
- 字体、字号和明暗模式使用现有原生 owner；自动重试、重试次数、思考开关、系统提示词写入预览身份的 `settings.json` 原有 `{settings: ...}` 格式并进入模型请求，保留其他字段。供应商仍由独立供应商文件负责。设置 schema/defaults 直接复用 `src-tauri/src/settings_config.rs`。
- 原版终端面板调用 Kivio `dock/terminal_runtime.rs`：真实 PTY、输入、调整大小、输出续接、关闭及退出清理。
- 原版附件暂存与预览接入 Kivio 附件事务：失败保留草稿、成功保存安全副本；图片/视频使用原有模型请求组装，普通文件保留工具读取路径。历史工具记录映射到原版工具卡。
- UI 的既有调用契约通过同进程 Rust service 适配；不启动 Zeron engine/daemon、账号认证、云同步、自动更新或 appshot 服务，不建立业务 IPC。

macOS 数据仍使用 `~/Library/Application Support/com.zmair.kivio.gpui.preview/`。对话及供应商文件在其 `conversations/` 下。不会自动改写正式 Kivio 数据目录。Study 扩展不在范围内。

## 尚未完成

这是一版能运行并发送文本的移植基底，不是全部业务完成版。对话 CLI Agent、工具执行循环、项目创建和工作区操作、插件、其他业务设置/云设备功能尚未适配；不可用的服务操作返回明确错误。原始 UI 中仍有对应入口和英文文案需要逐项收敛。普通文件附件的内容依赖工具读取，因此在工具循环接通前，不能宣称模型已经读过这些文件。屏幕引用与审查评论暂不支持发送。

Zeron fork 独有的场景模糊、边缘渐隐、圆角子树裁切、背景 alpha mask、文字倾斜模糊和消息尾部预留，在 Kit 0.8.0 上使用不透明浮层、硬裁切、普通图片/文字动画和尾随滚动。尚未达到全部视觉/滚动行为等价；不把缺失特效伪装成完整支持。

上游 tests/examples 随源码完整保留，尚未全部移植到 Kit 与 Kivio 服务契约；不能将全工作区测试视为已通过。当前边界检查为 `tests/kivio_port.rs`：无供应商时不创建对话、真实本地 HTTP 流式响应、落盘和重开、并发发送拒绝、运行中改名、取消保留部分内容、附件持久化/媒体校验/图片请求/读取范围、历史工具卡，以及真实 PTY 的早期输出回放、调整大小与关闭。原生窗口另行检查启动、历史渲染、17 类设置导航、基础/模型表单布局、混音器路由和明暗主题切换。设置回归检查覆盖未知字段保留、无效重试次数不落盘、自定义提示词进入真实本地 HTTP 请求、记忆配额、Hooks 并发修改拒绝及搜索字段合并/取消独立抓取源。2026-10-24：边界测试 5 项通过；新增抓取源清除用例先失败后通过。原有主题测试 44 项通过。原生窗口已检查主题应用及 Hooks 新建/空值错误；最新搜索页面通过编译，尚未完成实机交互检查。

### 侧栏页面迁移验证（2026-10-24）

- 作品页取消迁移并删除旧 React 页面；附件、聊天工具产物和用户文件保留。旧路由不可恢复，也不会误载为对话。前端路由/持久化/附件回归 125 项、Rust 路由回归 3 项通过。
- 定时任务：`scheduled_tasks/core.rs` 是两端共用的定义、计划、排队取消和运行记录负责人，原 Tauri 宿主保留平台接线。原生 `kivio/schedules.rs` 通过聊天发送准入锁协调普通发送和定时发送，不新增 IPC；沿用 `scheduled_tasks/` 数据目录，`tasks.json` 与运行记录保留未知字段；Cron 依赖锁定为旧端使用的 `croner 4.0.0`。
- `cargo check/build --manifest-path native/Cargo.toml --locked -p kivio-native` 通过。`scheduled_tasks` 5 项、`scheduled_task_forms` 2 项、`automation_catalog` 2 项及 `scheduled_task_execution` 1 项通过；后者经过真实 KivioService、本地 HTTP、对话/任务仓库，覆盖忙时排队、编辑撤销待发送运行、自动触发、实际发送与删除对话后停用且不重建。
- 独立 QA 应用使用 `/tmp/kivio-task-qa-home` 的合成对话，不改动正在运行的预览窗口或正式数据。已实际点击验证明暗主题、空列表、名称/提示词输入、空提示词错误、目标菜单、保存落盘、立即运行的缺供应商错误、运行记录、编辑/放弃确认、自动化空列表/正常记录/损坏记录。窄窗口实际受最小尺寸限制为约 902 × 721，列表与编辑界面已查看。
- 尚未验收：真实模型上游、外部 CLI 定时发送、多窗口发送协调、全部七种计划的 GUI 操作、完整键盘/IME/无障碍、字体缩放、Windows 和 Linux。自动化仅只读目录浏览，不能称为完整任务域迁移；插件与扩展页面尚未迁移，Study 不接入。
- 使用 `build-gpui-apps`、`gpui-kit`、`gpui-kit-design-guides`、`cua-driver`。参考锁定 Zeron 的 `crates/ui/src/settings/widgets.rs` 及 GPUI Kit 的 `crates/story/src/stories/{tabs_story,menu_story,dialog_story,input_story}.rs`，API 以实际 Kit/Component 0.8.0 源码为准；未找到可直接复用的定时任务业务页，故用 Kit 控件组合原生页面，不复制交互机制。

## 来源与验证依据

使用 `build-gpui-apps`、`gpui-kit`、`gpui-kit-design-guides` 与 `test-audit`。统一规则见 [工程规范](../docs/engineering-standards.md)。主要源码为 `task-GPUI/references/zeron/crates/ui`，锁定 `c04487045e5f934818ab21bf5b7b7fbed16ef3d6`；相关支持 crate 位于 `zeron-support/`。Kit API 依据锁定的 GPUI Kit/Base/Component 0.8.0、gpui-pre 0.3.9 实际依赖源码核对。

来源、原文件哈希及许可见 [UPSTREAM.json](zeron-ui/UPSTREAM.json)、[PORT.md](zeron-ui/PORT.md)、[LICENSE](zeron-ui/LICENSE)、[第三方声明](zeron-ui/THIRD_PARTY_NOTICES.md)。参考仓库本身不参与构建。浏览器源文件保留为来源记录，Wry、WebKit、WebView2 的依赖与编译入口已移除，网页交给系统浏览器。

### 侧边栏页面 UI 适配

插件/Skill/MCP 与助手/知识库/媒体/笔记已接入原生页面路由，可切换分类与筛选，复用 Zeron 标题、搜索输入、内容块和滚动。当前仅 UI 预览，不读取或修改已有数据；创建动作禁用并标明未接入，详情页和编辑器尚未搭齐。任务页统一外层布局与分段样式，保留既有业务接线。

使用 `build-gpui-apps`、`gpui-kit`、`gpui-kit-design-guides`；参考 Zeron `c044870` 的 `crates/ui/src/settings/widgets.rs`，API 核对实际 GPUI Kit/Component 0.8.0。原生构建通过；macOS 亮色窗口已检查插件、扩展、任务布局，逐项点击核对七个插件/扩展子页及定时任务/自动化的选中态和搜索占位。暗色、缩放、窄窗口与完整键盘路径尚未验收；UI 预览不代表业务迁移完成。
