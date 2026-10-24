# Kivio GPUI 客户端

当前启动入口使用 **完整移入的 Zeron UI 源码**，在 `zeron-ui/` 内适配 GPUI Kit 0.8.0。`src/main.rs` 直接启动它；旧 `src/shell.rs`、`src/preview.rs` 保留供迁移时提取业务接线，已不参与主程序编译。不是基于旧预览重新模仿 Zeron 的布局。

```sh
npm run dev
# 或
cargo run --manifest-path native/Cargo.toml --locked -p kivio-native
cargo build --manifest-path native/Cargo.toml --locked -p kivio-native
cargo build --manifest-path native/ocr-worker/Cargo.toml --locked
python3 native/scripts/bundle-preview.py
cargo test --manifest-path native/Cargo.toml --locked -p kivio-native --test kivio_port
```

macOS 开发包生成于 `native/target/Kivio Native.app`。退出当前应用后再启动新包。`npm run build` 构建原生 release 二进制；旧 Tauri 开发/构建命令保留为 `dev:tauri-legacy`、`build:tauri-legacy`，用于尚未迁出的源码参考，不是 GPUI 启动路径。此包尚非签名发布安装包。

## 当前实现边界

截至 `edba235a`，这是以完整移入的 Zeron 为基底、已接通部分 Kivio Rust 业务的原生客户端。**页面存在、配置可保存、实际执行可用是三个不同状态；17 项设置导航不代表 17 页迁移完成。** 以下结论来自当前代码核对，不能代替逐页实机验收。

### 启动与调用路径

```text
src/main.rs
  → zeron-ui::run_app → Kit Base Root → Zeron Shell
      ├─ 对话/附件/终端 → KivioService（同进程 RpcService 适配）
      │   ├─ ConversationStore：历史与变更
      │   ├─ chat_runtime → 原 Rust 模型适配器：流式请求
      │   ├─ external_agents::native / Zeron Harness → CLI 协议、续聊、工具事件与审批
      │   ├─ attachment_files：附件事务
      │   └─ terminal_runtime：PTY
      └─ 设置页面
          ├─ ProviderSettings → providers.json
          ├─ native_preferences → settings.json
          ├─ SettingsStore → ui-settings.json
          ├─ memory_files → L1/L2 文件
          └─ usage_ledger / conversation_library：只读查询
```

这里保留的是 Zeron UI 已有的调用契约，由 `zeron-ui/src/kivio.rs` 在同一进程接入 Kivio；并非已将全部 Zeron 服务替换完毕。没有启动 Zeron daemon 或业务 WebSocket，也没有桌面 WebView。`model-runtime/src/lib.rs` 通过路径包含复用 `src-tauri/src/` 的纯 Rust 模块；源码仍在旧目录不等于依赖 Tauri 运行时。

### 主界面业务

| 能力 | 当前实现 | 尚缺的部分 |
| --- | --- | --- |
| 作品页 | 已删除 React 页面、导航与页面专用 API，并移除原生侧边栏入口；旧路由不再恢复或误当作对话 | 取消 GPUI 迁移；保留聊天附件、工具产物及用户文件 |
| 插件与扩展页面 | 侧边栏接入原生页面；插件/Skill/MCP 与助手/知识库/媒体/笔记可切换，复用 Zeron 标题、分段、搜索输入、内容块和滚动；标明未接入、创建动作禁用 | 目前仅 UI 预览，不读取既有目录、不安装或保存数据；列表、详情与编辑器尚待逐项接入，不代表完整迁移 |
| 任务 | 原生列表/搜索/分页、七类定时计划、已有/新建对话绑定、启停、立即运行、运行记录/删除、保留草稿、标签记忆；复用原状态机与实际聊天发送 | 自动化目前仅查看列表/损坏记录提示，画布、编辑、导入导出和执行未迁移；通过聊天创建任务、侧栏小时钟、目标模型/项目/思考等级选择尚未接入 |
| 普通模型对话 | 真实供应商请求、流式文本/思考、停止、落盘、历史重开；重试和 Agent 系统提示词进入请求 | 完整 Agent 工具循环、Chat/Agent 模式语义仍未接齐 |
| 对话列表 | 读取、改名、归档/恢复、删除；项目索引读取 | 项目创建、工作区操作及完整分组操作 |
| 模型选择 | 模型 API 使用 Kivio 供应商与默认模型；CLI 恢复 Zeron 行内模型/推理菜单，并保存模型专有选项 | API 对话行内模型选择；CLI 第三方配置落地 |
| CLI 对话 | 恢复 Zeron 多 CLI/模型/推理选择器；Codex 使用原 app-server，Claude、Pi、Cursor、OpenCode、Grok、Hermes、Antigravity、Devin 使用已导入的 Harness；连接 Kivio 配置、附件、会话、工具结果、提问/审批、取消和退出回收 | 第三方供应商物化、跨轮常驻进程、运行中 steering/后台任务、导入/回退；Kivio 特有 Gemini/Kimi/dsh 尚未接入，不视为全迁移完成 |
| 工具调用 | CLI 的实际工具事件、结果与审批连接到工具卡和提问面板；历史记录落盘 | Kivio 内置 Agent 的工具派发、多轮工具循环 |
| 附件 | 暂存、校验、安全副本与事务保存；图片/视频进入既有请求组装 | 普通文件内容依赖工具读取，不能把“附件已保存”当作“模型已读取”；屏幕引用/审查评论未支持 |
| 终端 | 复用原 PTY：打开、输入、早期输出回放、尺寸变化、关闭和退出清理 | 不代表 CLI Agent 协议已接通 |

CLI 迁移继续遵守 [ADR-0001](../docs/adr/0001-imported-cli-conversations-stay-on-their-cli.md) 和 [ADR-0002](../docs/adr/0002-imported-history-is-a-snapshot.md)：CLI 对话不能退回普通模型执行，导入历史是显示快照，不能每次打开重读覆盖 Kivio 历史。

### 17 页设置核对

以下“可保存”只表示配置写入，除明确说明外不表示对应平台功能或工具已经执行。

| 页面 | 当前状态 | 仍需补齐 |
| --- | --- | --- |
| 基础 | 字体/字号可应用，重试开关/次数进入模型请求 | 界面语言、开机启动、托盘、窗口行为、通知、首次使用和权限 |
| 主题 | 原版 12 套明暗色板共用 `src/theme/builtins.json`；模式和色板同步 Zeron/Kit 控件 | 自定义主题编辑、导入/导出、透明度及完整预览交互 |
| 模型 | 供应商列表/详情；名称、接口、协议、密钥、模型及启停可编辑保存 | 原版模型管理、连接测试、请求配置二级页及细节交互 |
| 快捷键 | 展示原有键位 | 录制、冲突检测、恢复默认、系统注册与执行 |
| 翻译 | 恢复分组；语言与各类提示词可保存 | 模型选择、OCR/离线包、快速/替换翻译与浮窗执行；其余控件仍只读/禁用 |
| Lens | 语言、消息顺序、提示词可保存 | 截屏/问答窗口、模型选择、图片归档与相关开关的实际行为 |
| Kivio Agent | 用户名、响应语言和两类提示词可保存；Agent 系统提示词进入普通模型请求 | 完整工具循环、工作区选择、Chat 附加指令执行及其他配置消费 |
| 记忆 | L1/L2 文件读取、编辑、保存；复用原配额和内容校验 | 启用开关、目录操作、Agent 自动注入与检索调用 |
| 混音器 | 复用供应商文件中的模型用途配置入口 | 对照原版补全用途、子代理/Advisor 配置与执行端消费 |
| 本地 CLI 管理 | 占位说明与已有配置数量 | 原版检测/列表/详情、路径、版本、模型、供应商与账号操作 |
| 电脑操控 | 占位说明 | CuaDriver、Playwright、ego lite 的真实检测、安装/更新、配置与执行 |
| Hooks | 8 类事件、命令/HTTP 编辑、启停/删除及冲突检查可保存 | 对话生命周期事件派发尚未接入；保存不会执行脚本 |
| 对话库 | 五个范围、标题/正文检索、排序、分页、打开、归档/恢复 | 项目/分组导航、密度与批量操作等原版能力 |
| 网络搜索 | 14 个服务商详情和默认选项可保存，按变更字段合并 | Agent/Lens 搜索执行、测试交互、TinyFish OAuth 授权入口 |
| 使用统计 | 真实账本汇总、趋势与调用明细 | 日历热图、请求调试、更多筛选与清理操作 |
| 备份与同步 | 占位说明 | 配置/全量导出、导入、信任选项与 WebDAV |
| 关于 | 原生客户端基本信息 | 原版品牌/版本展示、应用更新和模型库更新 |

### 状态与源码归属

| 对象 | 唯一写入/查询负责人 | 位置 |
| --- | --- | --- |
| 对话、项目 | 原 Rust `ConversationStore` / `group_catalog` | `zeron-ui/src/kivio.rs` 接线；`model-runtime/src/lib.rs` 复用源文件 |
| 供应商、默认模型用途 | `ProviderDocument` | `src/providers.rs`；`conversations/providers.json` |
| Kivio 业务设置 | `native_preferences` | `model-runtime/src/native_preferences.rs`；原 `{settings: ...}` 文档格式，保留未知字段 |
| 窗口、字体、原生外观 | `SettingsStore`、`appearance`、`typography` | `zeron-ui/src/`；`ui-settings.json` |
| 设置页草稿、请求、订阅 | 各页面 Entity | `zeron-ui/src/kivio_settings.rs`、`zeron-ui/src/settings/{hooks,search,memory,library,usage}.rs` |
| 记忆、历史检索、用量 | 原 Rust 领域实现 | `src-tauri/src/chat/{memory_files,conversation_library}.rs`、`src-tauri/src/usage_ledger.rs` |

macOS 使用 `~/Library/Application Support/com.zmair.kivio.gpui.preview/`，不自动读取/覆盖正式版配置。正式数据迁入、身份切换与发布安装包尚未完成。Study 整个扩展不在本次范围。

## 待收敛的实现

1. **执行入口逐项核实。** CLI/模型/Skill/命令列表已改为调用真实 Harness，使用与运行一致的工作目录；部分非 CLI 的焦点/同步操作仍直接返回空成功，不能视为能力可用。
2. **设置保存与执行消费逐项对齐。** 翻译、Lens、搜索、记忆、Hooks 等不能以写入成功作为功能完成条件。每项应验证关闭重开后的值及实际执行结果。
3. **旧设置入口仍有残留。** `shell.rs` 保留未使用的原 Zeron 页面 Entity/订阅字段；`kivio_settings.rs` 还保留已被独立页面接管的 Memory/Hooks/Search/Usage 占位分支。应沿实际路由逐项清理，不能通过删掉整套 Zeron 前端解决。
4. **参考与运行源码要分清。** `native/src/shell.rs`、`native/src/preview.rs` 是未参与当前主程序的旧预览；`task-GPUI/references/` 是锁定参考；当前运行界面是 `native/zeron-ui/`。保留参考源码，不恢复已取消的任务/进度体系。
5. **视觉与平台验收尚不完整。** Zeron fork 的场景模糊、边缘渐隐、圆角子树裁切、背景 alpha mask、文字倾斜模糊等使用 Kit 0.8.0 的现有替代方式。不能宣称视觉/滚动等价；Windows、IME/焦点、无障碍及发布包需独立验收。

设置页面由独立工作负责。执行端仍需完成 CLI 第三方配置、常驻会话/steering 与特有协议，以及 Kivio Agent 工具循环（带动搜索、记忆、Hooks、普通文件读取）、平台能力、备份和发布更新。每次沿现有 Rust 业务负责人接线，不另建平行业务实现。

## 验证记录与限制

侧边栏页面视觉适配：使用 `build-gpui-apps`、`gpui-kit`、`gpui-kit-design-guides`；参考 Zeron `c044870` 的 `crates/ui/src/settings/widgets.rs`，API 核对实际 GPUI Kit/Component 0.8.0。原生构建通过；macOS 窗口已核对插件与任务页的亮色布局。暗色、缩放、窄窗口、完整键盘路径与各子页的自动化交互尚未全部验收；本轮不以截图代替业务接线验证。

2026-10-24 CLI 接线验证：

- `kivio_cli` 两项进程边界测试覆盖 Codex、Claude stream-json 和 ACP；`kivio_port` 六项现有回归通过，包括真实 PTY 的重放、缩放和关闭。
- CLI 测试启动独立 stdio 子进程，经过真实 KivioService、原 Codex 协议及文件仓库，覆盖实际读取暂存附件、最终文本修正、工具结果落盘、重开续聊、`turn/interrupt`、退出回收，以及通过导入的提问契约提交权限拒绝。Claude/ACP 同样覆盖提问回答、原生会话恢复及进程回收；恢复失败不会新建会话。未调用真实模型上游，不代表真实账号逐一验收通过。
- 驱动回归：ACP 75 项、Claude 25 项（2 项真实上游测试未运行）、OpenCode 23 项、Pi 14 项、Skill 发现 15 项通过。修复全局 Skill 数量耗尽扫描额度而遮蔽项目 Skill 的问题。
- 独立 GPUI 原生窗口已展开多 CLI/模型菜单，核对本机 Claude Code、Codex、Pi、OpenCode、Grok、Hermes 条目与推理选项；没有通过 GUI 发送真实上游请求。
- 模型选项随 Kivio 会话保存；已有会话不允许切换 CLI；ACP/OpenCode/Pi 恢复失败停止，不静默丢弃历史另起会话。
- CLI 会话绑定存放于预览数据目录的 `external-agent-sessions/`，续聊失败不会改走模型 API 或静默创建新线程。新会话使用 CLI 自身登录及原生设置；Kivio 第三方供应商配置尚未接入时明确报错。
- `node scripts/check-architecture.mjs` 通过。没有重新编译旧 Tauri 客户端，也没有修改设置页面。
- 此轮参考 `task-GPUI/references/zeron/crates/harness/src/process/owned.rs`、GPUI Kit `examples/dock/src/main.rs`；复用 `src-tauri/src/external_agents/session/codex_app_server.rs` 及抽取的进程/历史匹配/命令解析代码，Kit API 以实际 0.8.0 依赖为准。

此前设置迁移的已执行记录：

- `cargo check/build --manifest-path native/Cargo.toml --locked -p kivio-native` 通过；有既存编译警告，不等于全工作区无警告。
- `cargo test --manifest-path native/Cargo.toml --locked -p kivio-native --test kivio_port -- --test-threads=1`：5 项通过，覆盖真实本地 HTTP 对话、持久化/取消/附件、PTY、记忆与设置保存。
- 搜索“独立抓取源改回跟随搜索”的回归用例先失败，修复后定向通过；最后修复后没有重新跑完整五项。
- `npm test -- src/theme/theme.test.ts`：原主题测试 44 项通过。
- 安装版 Kivio 与原 `src/settings/` 代码共同对照；没有为参考重新编译旧 Tauri 应用。
- 原生窗口已检查设置导航、部分表单布局、主题应用、Hooks 新建/空值错误。最新搜索页面尚未做实机交互验收；未完成全 17 页逐项验收。

上游 tests/examples 随源码保留，尚未全部适配 Kit/Kivio 契约。以上记录不代表全工作区测试、真实 CLI 对话、Agent 工具执行、旧 Tauri 客户端或跨平台发布验证通过。

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
