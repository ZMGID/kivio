# Gemini 工具 schema 约束型 anyOf 剥离（Vertex 校验兼容）

## Goal

Vertex AI Gemini 供应商在工具列表含 `present_artifacts` 时整请求被拒：
`Unable to submit request because 'present_artifacts' functionDeclaration 'parameters' schema didn't specify the schema type field`。
修复 `chat/model/gemini.rs::normalize_gemini_schema`，使发给 Gemini 的 functionDeclaration schema 通过 Vertex 严格校验。

## 根因

`present_artifacts` 的 input_schema（`mcp/types.rs:548`）顶层带约束型 anyOf：

```json
"anyOf": [ { "required": ["artifact_ids"] }, { "required": ["paths"] } ]
```

`normalize_gemini_schema` 只折 nullable anyOf（`[T, null]` 两分支且一支为 `type:null`），此形态不匹配 → anyOf 原样透传。Vertex 要求 anyOf 每个分支是带 `type` 的完整 schema，提交前拒绝整个请求。官方 generativelanguage API 校验松，未暴露。

## Requirements

- 在 `normalize_gemini_schema` 中剥掉"约束型组合子"：`anyOf`/`oneOf`/`allOf` 中所有分支都不含 `type` 的（纯 `required` 组合约束），整个组合子键移除，其余字段（`type`/`properties`/…）保留。
- 保留现有 nullable anyOf 折叠行为不变。
- 含 `type` 的多态 anyOf 分支（如 `[{type:string},{type:integer}]`）维持现状透传（Vertex 支持带 type 的 anyOf）。
- "至少提供一个" 语义不迁移进 schema——工具执行侧（`present_artifacts` 的执行逻辑）已有/应有参数校验兜底，本任务不改工具行为。
- MCP 外部服务器工具的 schema 也走同一归一化，同样受益。
- 补 Rust 单测：约束型 anyOf 被剥、nullable anyOf 仍折叠、带 type 的 anyOf 保留。

## Acceptance Criteria

- [ ] `gemini_function_declarations(&[present_artifacts])` 输出的 `parameters` 无 `anyOf` 且顶层有 `"type":"object"`。
- [ ] 现有 gemini.rs 相关单测全绿；新增单测覆盖三种 anyOf 形态。
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` 通过（对照基线）。

## Notes

- 父任务：`.trellis/tasks/07-22-gemini-compat-fixes`。
- 轻量任务，PRD-only。
