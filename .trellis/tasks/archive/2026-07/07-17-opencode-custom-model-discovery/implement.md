# Implementation Plan

1. 增加 OpenCode 原生模型输出解析函数及单元测试。
2. 在 detection 层加入 OpenCode 专用原生 probe，并实现 native -> ACP -> static fallback 顺序。
3. 为检测函数增加显式 cwd；让命令入口和实际运行入口使用 `resolve_effective_cwd`。
4. 把全量检测与模型缓存改为 cwd-scoped，并同步上下文估算读取键。
5. 给三个聊天顶栏选择器传入当前会话 ID，扩展前端 API 的可选参数。
6. 增加失败、空输出、重复模型、自定义 provider 和缓存隔离测试。
7. 运行 Rust 格式化、external_agents 测试和前端类型检查。

## Risk Points

- `src-tauri/src/external_agents/detection.rs`
- `src-tauri/src/external_agents/session/acp.rs`
- `src-tauri/src/external_agents/commands.rs`、`src-tauri/src/state.rs`（仅项目级 cwd/cache 纳入范围时）
- `src-tauri/src/external_agents/workspace.rs`
- `src-tauri/src/external_agents/run.rs`
- `src-tauri/src/chat/commands/context.rs`
- `src/chat/api.ts`
- `src/chat/RuntimePicker.tsx`
- `src/chat/PermissionPicker.tsx`
- `src/chat/Chat.tsx`

## Validation

- `cargo test --manifest-path src-tauri/Cargo.toml external_agents::detection`
- `cargo test --manifest-path src-tauri/Cargo.toml external_agents::session::acp`
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
- 运行 `package.json` 中现有的 TypeScript 类型检查命令。
