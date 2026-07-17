# Implementation Plan

1. 激活并完成 `07-17-pi-epipe-shutdown`。
2. 激活并完成 `07-17-opencode-custom-model-discovery`。
3. 运行 external_agents 全量 Rust 测试、格式化、前端类型检查和 Trellis 质量检查。
4. 核对父任务全部验收标准后归档子任务及父任务。

## Validation Gate

- 两个子任务的针对性测试均通过。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
- 项目现有 lint/type-check 命令通过。
- 不修改用户已有的 `website/DEPLOY.md` 与 `website/deploy.sh`。
