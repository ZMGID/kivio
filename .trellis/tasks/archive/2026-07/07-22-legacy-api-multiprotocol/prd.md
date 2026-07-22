# 翻译/截图/Lens 旧调用路径接入多协议模型适配器

## Goal

修复：`api_format=gemini`（及 anthropic_messages / openai_responses）的供应商被选为翻译器/截图翻译/Lens 模型时报
`404 path not found: /gemini/v1beta/chat/completions`。让这些旧调用路径尊重供应商的 `api_format`，与聊天行为一致。

## 根因

`api.rs` 的非聊天调用点（约 5 处：`api.rs:686`/`766`/`957`/`1252`/`1357`，覆盖文本翻译、OCR/vision、Lens explain、`stream_translate_combined` 等）全部硬编码
`format!("{}/chat/completions", base_url)` + `.bearer_auth(key)`，从不读取 `provider.api_format`（`settings.rs:65`）。
聊天正常是因为它走 `chat/model/` 的四个 peer adapter。

## Requirements

- 方向：把 `api.rs` 的翻译/OCR/Lens 调用迁到 `chat/model/` 的 `LanguageModelProvider` 抽象上（构建 `GenerateRequest`，复用 openai/anthropic/gemini/responses 四个适配器），而不是在 `api.rs` 内再实现一套协议分支。
- 流式路径（`lens-stream`/`lens-translate-stream`）需保留现有 Tauri 事件契约（payload 形状不变，含 `<<<ORIGINAL>>>` 分隔的 combined translate 协议、`delta.reasoning_content` 推理增量）。
- 取消语义保留：`explain_stream_generation` 代际取消要继续生效。
- 多 key failover（`send_with_failover`）与 usage 记录（`record_api_usage`，source/operation 维度）不能丢。
- vision/OCR 调用（带图请求）需要各协议的图像编码走对应 adapter 的 message 组装。
- 不改变 OpenAI 供应商的现有行为（回归风险最低优先）。

## Acceptance Criteria

- [ ] `api_format=gemini` 供应商作为截图翻译/Lens/翻译器模型：请求命中 `{base}/models/{model}:streamGenerateContent?alt=sse`，功能端到端可用。
- [ ] `api_format=anthropic_messages` / `openai_responses` 同理不再 404。
- [ ] OpenAI 供应商所有翻译/截图/Lens 流程回归无变化（手动 smoke）。
- [ ] 取消、failover、usage 统计行为不回退。
- [ ] `cargo test` + `npm run lint` + `npm run typecheck` 全绿。

## Notes

- 父任务：`.trellis/tasks/07-22-gemini-compat-fixes`。
- 复杂任务：`task.py start` 前需补 `design.md`（GenerateRequest 组装、流事件桥接层设计）+ `implement.md`。
- 无实现依赖于兄弟任务 `07-22-gemini-anyof-strip`，但排序在其后。
