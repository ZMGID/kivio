# 用户消息支持编辑并重新生成

## Goal

用户可以修改自己已发送的提问：用户气泡操作区新增编辑按钮，改完内容后截掉其后的所有消息并重新生成回复（ChatGPT/Claude 同款交互）。调研结论见 journal session（2026-07-02）：`chat_regenerate_message` 已具备"以 user 消息为基点截断重生成"的全部脏活处理，本任务是在其上加"先替换内容"一步。

## Requirements

### R1（后端）：`chat_regenerate_message` 扩展 `new_content: Option<String>`
- 有值且目标消息 role=="user" 时：先 trim 校验非空 → 替换 `messages[idx].content` → 再走既有截断+重生成流程。
- 摘要失效标记：改了内容时用 `mark_summary_stale_if_needed(&mut conversation, idx)`（内容变了，覆盖到该条的摘要即失效）；未改内容维持现状 `idx + 1`。
- 附件保留不动：`compose_user_content_for_api` 用新文本 + 原附件重组 API 内容（既有逻辑，无需改）。
- `new_content` 对 assistant 消息无意义：带值但目标是 assistant → 返回错误（不静默忽略）。
- **不放宽 `chat_update_message`** 的 assistant-only 限制——避免"改了历史但不重生成"的不一致状态。

### R2（前端）：用户气泡编辑态
- 用户气泡按钮区（复制/删除之间）加 ✏️ 编辑按钮；仅在会话空闲（非 in-flight）时可用。
- 点击进入编辑态：气泡原地变 textarea（复用 assistant 编辑的交互模式，样式对齐右侧用户气泡），预填原文。
- 确认按钮文案"保存并重新生成"（明确告知会丢弃其后的回答）；取消恢复原文退出。
- 提交走 `handleRegenerateMessage` 扩展版（携带新内容）：复用既有乐观截断 + 流式衔接 + busy 拒绝提示。

### R3（API 层）
- `src/api/tauri.ts` 的 `regenerateMessage` 加可选 `newContent` 参数，透传后端。

## Acceptance Criteria

- [x] AC1：编辑一条用户消息并确认 → 该消息内容更新、其后所有消息被移除、自动开始重新生成，新回复基于新问题。
- [x] AC2：编辑带附件的用户消息 → 附件保留，重新生成的请求包含原附件 + 新文本。
- [x] AC3：会话有 run 在跑时编辑入口不可用（disabled）；绕过 UI 直接调命令时后端 busy 拒绝（既有 `ChatSendReservation`）。
- [x] AC4：编辑被摘要覆盖范围内的消息 → 摘要标 stale（下次发送不再注入过期摘要）。
- [x] AC5：多模型一问多答组被截断后 `group_selections` 清理正常（既有逻辑不回归）。
- [x] AC6：`new_content` 指向 assistant 消息 → 明确报错；空内容 → 报错"消息内容不能为空"。
- [x] AC7：`npm run typecheck` / `npm run lint` / `npm test` 全绿；Rust 侧 `cargo check --lib --tests` 干净，新逻辑有单测（cargo test 环境问题时用 harness，提交注明）。

## Notes

- 范围外：消息分支树（多版本问题并存切换）、编辑 assistant 消息行为（已有）、外部 agent 会话。
- 交互参考：ChatGPT/Claude 的"编辑提问"= 编辑 + 截断 + 重发，本项目无分支模型，截断语义与既有"重新生成"一致。
