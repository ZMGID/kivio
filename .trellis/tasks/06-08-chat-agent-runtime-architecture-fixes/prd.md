# 修复 Chat agent runtime 架构问题

## Goal

把 Chat agent runtime 从“能调用工具”推进到更接近成熟 agent runtime 的安全契约：确认已修复工具轮次上限，并补齐剩余高价值后端缺口，包括工具参数校验、MCP 元数据保留、权限/并行调度语义改进，以及基础可观测字段。

## What I Already Know

- 用户已修过第一个问题，即工具循环上限。
- 当前代码里 `ChatToolsConfig.max_tool_rounds` 已有默认值和 clamp。
- `run_agent_loop` 已在达到轮次上限后追加 `step_limit_system_message()` 并进入 synthesis。
- 架构审查里剩余问题集中在 `src-tauri/src/chat/agent/**`、`src-tauri/src/mcp/**`、`src-tauri/src/chat/types.rs` 和前后端序列化边界。

## Requirements

- 保持现有工具调用顺序契约：并行执行不能改变 tool result replay 顺序。
- 工具执行前做集中参数 schema 校验；校验失败不得进入审批或真实工具执行。
- MCP 工具定义保留官方 `annotations`、`outputSchema` 等元数据；MCP 结果保留 `structuredContent`。
- 权限与并行调度优先使用 MCP annotations/工具元数据，不能只靠工具名猜敏感性。
- 给工具调用记录增加基础 trace 元数据，方便后续诊断。
- 尽量保持前端兼容；新增字段使用 serde/default/TS optional。

## Acceptance Criteria

- [ ] 工具轮次上限修复经测试覆盖或现有测试证明。
- [ ] schema 无效的工具参数返回 error tool result，且不执行真实工具。
- [ ] MCP read-only annotations 可让非敏感 MCP 工具自动审批/可并行；destructive/open-world 等 annotations 会标记为敏感。
- [ ] MCP `outputSchema` / `annotations` / `structuredContent` 不在后端边界丢失。
- [ ] Rust targeted tests 覆盖关键新行为。
- [ ] `npm run typecheck`、`npm run lint`、`cargo test --manifest-path src-tauri/Cargo.toml` 在可行时运行并记录结果。

## Out Of Scope

- 不重写整个 agent loop。
- 不迁移到 OpenAI Responses API 或某个第三方 SDK。
- 不做完整 trace UI。
- 不大改前端设置页交互。

## Technical Notes

- Backend guideline: `.trellis/spec/backend/agent-runtime.md`
- Key files:
  - `src-tauri/src/chat/agent/loop_.rs`
  - `src-tauri/src/chat/agent/execute.rs`
  - `src-tauri/src/mcp/types.rs`
  - `src-tauri/src/mcp/client.rs`
  - `src-tauri/src/chat/types.rs`
  - `src/api/tauri.ts`
  - `src/chat/types.ts`
