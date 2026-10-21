# Kivio 内置扩展

本目录保存随桌面应用一起构建的可选功能包。用户在「扩展 → 管理扩展」启用；未启用时只读取清单，不加载页面或启动原生服务。现有 Agent 插件市场（Skills、MCP 等）保持原有用途。

- [`sdk/`](sdk/README.md)：API v1、Hook 契约、宿主聊天与 UI 能力。
- [`study/`](study/README.md)：第一个实际扩展，包含阅读、文献、笔记、原生任务和 Word 集成。
- `src/extensions/`：清单发现、前端实例生命周期、管理界面。
- `src-tauri/src/extensions.rs`：默认值、持久设置、原生生命周期与命令准入的唯一负责人。

## 添加扩展

1. 新建 `extensions/<id>/manifest.json` 和 `index.tsx`。复制 Study 的结构，填写唯一的小写 ID、名称、说明、版本和 `apiVersion: 1`。
2. 通过 `defineExtension({ Page, activate })` 导出模块。`activate` 只注册 Hook；资源在 `activated` 或用户显式操作中启动，响应上下文的 `AbortSignal` 清理。
3. 页面通过 SDK 获取共享控件与聊天能力。扩展不能导入宿主领域私有文件，也不能互相深层导入；TypeScript 架构检查覆盖本目录；原生依赖使用 `sdk/native.rs` 的公开导出。
4. Vite 和 Rust 构建都从相同的清单目录发现扩展，不需要修改侧栏或聊天页面。规范入口为 `#chat/extensions/<id>`，历史地址由清单的 `legacyRoutes` 声明。
5. 纯前端扩展无需原生注册。需要 Rust 的包在本目录保存 `native/`，由应用显式编译注册，接入 `extensions.rs` 的生命周期与命令分发。原生命令在执行期间必须持有宿主运行租约，关闭会等待租约结束，再停止后台任务。

此版本是**源码级内置扩展 SDK**：与应用同版本编译，并非已经支持下载 ZIP、执行任意第三方代码或独立自动更新。内置扩展是受信任代码，SDK 的能力范围不等于安全沙箱。新增原生能力需代码审查与重新构建，不能把任意命令名透传给扩展。

开发与验收继续遵循 [统一工程规范](../docs/engineering-standards.md)，不另立工程规则。
