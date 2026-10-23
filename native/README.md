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
- 输入框的「模型 / 供应商」打开已有 Kivio 供应商编辑器。新对话使用已保存的默认模型，已有对话继续使用其保存的供应商和模型。尚未恢复原版 Zeron 的行内模型选择交互。
- UI 的既有调用契约通过同进程 Rust service 适配；不启动 Zeron engine/daemon、账号认证、云同步、自动更新或 appshot 服务，不建立业务 IPC。

macOS 数据仍使用 `~/Library/Application Support/com.zmair.kivio.gpui.preview/`。对话及供应商文件在其 `conversations/` 下。不会自动改写正式 Kivio 数据目录。Study 扩展不在范围内。

## 尚未完成

这是一版能运行并发送文本的移植基底，不是全部业务完成版。CLI Agent、工具循环、附件发送、项目创建和工作区操作、终端接线、插件、其他设置/云设备功能尚未全部适配；不可用的服务操作返回明确错误。原始 UI 中仍有对应入口和英文文案需要逐项收敛。已有附件/工具记录保留在 Kivio 数据中，但当前历史适配只呈现文本和推理。

Zeron fork 独有的场景模糊、边缘渐隐、圆角子树裁切、背景 alpha mask、文字倾斜模糊和消息尾部预留，在 Kit 0.8.0 上使用不透明浮层、硬裁切、普通图片/文字动画和尾随滚动。尚未达到全部视觉/滚动行为等价；不把缺失特效伪装成完整支持。

上游 tests/examples 随源码完整保留，尚未全部移植到 Kit 与 Kivio 服务契约；不能将全工作区测试视为已通过。当前边界检查为 `tests/kivio_port.rs`：无供应商时不创建对话、真实本地 HTTP 流式响应、落盘和重开、并发发送拒绝、运行中改名、取消保留部分内容。原生窗口另行检查启动、历史渲染、导航和供应商编辑器。

## 来源与验证依据

使用 `build-gpui-apps`、`gpui-kit`、`gpui-kit-design-guides` 与 `test-audit`。统一规则见 [工程规范](../docs/engineering-standards.md)。主要源码为 `task-GPUI/references/zeron/crates/ui`，锁定 `c04487045e5f934818ab21bf5b7b7fbed16ef3d6`；相关支持 crate 位于 `zeron-support/`。Kit API 依据锁定的 GPUI Kit/Base/Component 0.8.0、gpui-pre 0.3.9 实际依赖源码核对。

来源、原文件哈希及许可见 [UPSTREAM.json](zeron-ui/UPSTREAM.json)、[PORT.md](zeron-ui/PORT.md)、[LICENSE](zeron-ui/LICENSE)、[第三方声明](zeron-ui/THIRD_PARTY_NOTICES.md)。参考仓库本身不参与构建。浏览器源文件保留为来源记录，Wry、WebKit、WebView2 的依赖与编译入口已移除，网页交给系统浏览器。
