# Implementation Plan

1. **配置模型与前端设置**
   - 后端新增 `working_directory` 默认值和旧 `workspace_roots` 单项迁移。
   - 前端类型、normalize/default 和 SettingsShell 改为单目录输入、选择、恢复默认。
   - 增加配置迁移与序列化测试。

2. **统一 NativeToolWorkspace 路径语义**
   - 增加普通对话默认目录字段和构造器。
   - 相对 read/write/list/glob/search 与缺省 run_command cwd 统一解析到默认目录。
   - 显式绝对/`~/` 路径保持不受限制。
   - 更新 registry 的对话/项目工作区构造和路径测试。

3. **合并交付目录**
   - `SandboxExportContext` 携带实际输出目录。
   - run_python 产物导出到当前工作区。
   - 普通对话 write_file 工作台内写入继续生成文件卡。
   - 移除 outputs 固定根和 16 文件裁剪；更新打开/定位校验及提示词/工具描述。

4. **迁移与删除生命周期**
   - 实现无覆盖目录合并、legacy outputs 懒迁移、artifact path 重写。
   - 设置根目录变化前迁移普通对话工作台。
   - 删除普通对话时安全删除当前工作台；项目对话不删除项目目录。
   - 补充冲突、跨根、路径逃逸与项目保护测试。

5. **验证**
   - Rust 定向单测：settings、native_tools、sandbox exports、chat storage/commands、prompt。
   - 前端 typecheck / 相关测试。
   - Windows 脚本执行 cargo tests，确认 run_command cwd 与 Git Bash/PowerShell 选择不回归。
   - 全局搜索确认运行时代码不再写入 `Kivio/outputs`，仅保留 legacy migration 常量/测试。

## Risk / Rollback Points

- 设置保存期间迁移涉及用户文件：必须先预检冲突，任何失败不得覆盖目标或删除源目录。
- artifact path 重写必须覆盖 message.artifacts 与 tool_calls.artifacts。
- 不能把工作目录误实现为路径限制；绝对路径和 `~/` 回归测试必须保留。
- 删除对话必须先判定是否项目对话，绝不能删除绑定项目根目录。

## Verification Result (2026-07-11)

- [x] `npm run typecheck`
- [x] `npm run lint`
- [x] `cargo check --manifest-path src-tauri/Cargo.toml`
- [x] `cargo test --manifest-path src-tauri/Cargo.toml --lib conversation_workspace_tests`
- [x] `cargo test --manifest-path src-tauri/Cargo.toml --lib native_tools::sandbox_exports::tests`
- [x] `cargo test --manifest-path src-tauri/Cargo.toml --lib native_tools::tests`
- [x] `cargo test --manifest-path src-tauri/Cargo.toml --lib mcp::registry::tests`
- [x] `cargo test --manifest-path src-tauri/Cargo.toml --lib settings::tests::sanitize_native_tools`
- [x] `cargo test --manifest-path src-tauri/Cargo.toml --lib chat::agent::prepare::tests`
- [x] `git diff --check`
- [x] Global search confirms `Kivio/outputs` remains only as the legacy migration constant.

Full `cargo test --manifest-path src-tauri/Cargo.toml --lib` completed with 1345 passed, 8 ignored, and 8 failures. One task-local Windows canonical-path assertion was corrected and now passes. The remaining 7 failures are reproducible pre-existing failures in chat compaction/case-insensitive tool tests and macOS PATH tests; they are outside this task's changed behavior.

Review fix: removed internal artifact `meta.json` bookkeeping from the active workbench/project root. Artifact metadata already lives in conversation records, so generated outputs no longer pollute user work directories.

