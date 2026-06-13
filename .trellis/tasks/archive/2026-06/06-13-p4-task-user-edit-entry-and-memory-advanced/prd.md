# P4 — Memory Search（L2 长期记忆检索）

## Goal

给 chat memory 的 L2 长期记忆加一个 `memory_search` 检索工具：现在 L2 只能"精确标题匹配"（匹配不上就报找不到），升级为按关键词/token 检索、返回最相关的若干片段。这是 P4 路线图里唯一进本期的项——经与用户确认，**task 用户编辑入口不做**（工作清单保持 agent 维护、用户只读），Memory 的固化钩子 / project scope 也不做。

## Technical Approach

- 新增原生工具 `memory_search`（`src-tauri/src/chat/memory.rs::tool_search` + `mcp/types.rs::native_memory_search_tool` + `mcp/native_registry.rs` 注册一条）。
- 参数：`query`（必需）、`maxResults`（可选，默认 5）、`layer`（可选，默认 `l2`；L1 很小且总是注入 prompt，检索主要面向 L2）。
- 实现：读取目标层内容 → 按 `#`/`##` 标题切块 → 对每块用 **query token 重叠打分**（小写化；标题命中加权）→ 按分数降序返回 top-N `{heading, snippet}`（片段截断）；无命中返回明确提示。**纯字符串/token 匹配，不引向量库、不加新依赖。**
- registry 元数据与 `memory_read` 对齐：`read_only=true`、`parallel_safe=false`（与 memory_read 一致的有意选择）、`bypasses_approval=true`、`enabled` gated by `memory_enabled`，经 `list_native_builtin_tool_defs` 暴露。
- 在 memory 工具描述/提示里提一句 `memory_search`，让模型知道"找不准标题时用 search"。

## Decision (ADR-lite)

- **Context**：P4 原含 task 用户编辑 + memory 三项（search/固化/project scope）。
- **Decision**：经 brainstorm，用户拍板**只做 `memory_search`**。task 工作清单保持用户只读（不加编辑）；memory 固化钩子、project scope、自动固化均不做。
- **Consequences**：范围最小、风险最低、无新依赖；其余 P4 项如需要后续单独起任务评估。

## Research References

- 现状摸查见下方 Technical Notes（Memory 系统现状段）——本期实现直接基于该现状，无需额外 web 调研。


## What I already know

- todo/task 线协议名仍是 `todo`（`AgentTodoItem`/`todo_write`/`todo_update`/`agent_todo_state`），概念上是任务（subject/description/四态/依赖边/owner，P2-C）。per-conversation 隔离。
- 现状：task 用户只读（spec「Agent Todo Runtime State」：user read-only this phase，编辑入口 deferred to P4）。
- owner 字段：P3 已定为"父 orchestrator 自上而下委派标记"。
- memory：有 `memory_read`/`memory_modify` 原生工具（gated by chat_memory.enabled），实现于 `chat/memory.rs`。

## Assumptions (temporary)

- 两块可拆成子任务分别交付；可能先做其中一块作为 MVP。
- Task 编辑入口在 Chat 的 todo 面板/设置里加编辑能力（具体形态待定）。

## Open Questions

- MVP 范围：P4 两块都做，还是先做一块？哪块优先？
- Task 编辑：作用域（per-conversation 现状 vs project 级）？编辑能力到哪（增删改状态/内容/依赖/owner）？
- Memory 进阶三项（固化钩子 / memory_search / project scope）哪些进 MVP？

## Requirements (evolving)

- **Memory：仅实现 `memory_search`**——把 L2 长期记忆从"只能精确标题匹配"升级为可检索（关键词/token 重叠打分，返回 top-N {标题+片段}），纯字符串匹配、不引向量库/新依赖。仿 `memory_read` 注册到 native registry。
- Task 工作清单：**保持现状**（agent 维护、用户只读、简单），不加用户编辑。（"把 task 整一下"的具体含义待用户澄清 A/B。）

## Acceptance Criteria (evolving)

- [ ] 新增 `memory_search` 原生工具，gated by `chat_memory.enabled`；模型可用关键词检索 L2 并拿到最相关片段。
- [ ] 现有 `memory_read`/`memory_modify` 行为不回归。
- [ ] cargo test + typecheck/lint 全绿；spec 更新。

## Out of Scope (explicit)

- ❌ Task 用户编辑入口（用户明确：工作清单很简单、用户不可编辑、保持只读现状）。
- ❌ Memory 会话末固化钩子。
- ❌ Memory project scope（按项目隔离）。
- ❌ Memory 自动固化 / 每轮自动写。


## Technical Notes

### Task/todo 系统现状（Explore 摸查）
- 数据模型 `AgentTodoItem{id,content,description?,status(4态),blocks,blocked_by,owner?}` + `AgentTodoState`，存 `Conversation.agent_todo_state`，**per-conversation 隔离**（P2-C 明确废弃 project 级共享，无残留）。
- agent 侧：`todo_write`/`todo_update`（native, enabled=false 单独 append, bypasses_approval）→ `handle_conversation_tool_call`（load→apply→`normalized_state` 校验→save→`emit_chat_todo_state`）。
- **无任何用户侧 command 能改 todo**；前端 `src/chat/AgentTodoIndicator.tsx` 只读，听 `chat-todo`（`api.onChatTodo`）。
- spec「Agent Todo Runtime State」: "user-read-only this phase (edit entry deferred to P4)"。
- 加编辑入口最小改动：复用 `normalized_state` + `emit_chat_todo_state`（0 改动），新增 ~2 个 Tauri command（仿 handle_conversation_tool_call），前端面板加编辑 UI + 2 个 api 包裹。Rust ~50 行 / TS ~90 行 / spec 2-3 段。

### Memory 系统现状（Explore 摸查）
- 存 `app_data/chat-memory/{L1.md,L2.md}`，**全局**（无 conversation/project 维度）；L1 5KB cap 注入 system prompt（`l1_prompt_block`→`chat_memory_prompt_for_request`→`build_chat_system_prompt`）。
- `memory_read`(layer l1/l2, L2 query=精确标题匹配) / `memory_modify`(append/replace/remove/archive)，native bypasses_approval，gated by `chat_memory.enabled`(默认 false)。
- 三项缺口最小路径 + 独立性/复杂度：
  - **memory_search**（最独立/低复杂）：现 query 仅精确匹配；新增检索工具（模糊/多结果），仿 memory_read 注册。
  - **会话末固化钩子**（中）：现完全没有；挂 `complete_assistant_reply` 后的 command 或 `finalize` 操作，默认关/手动触发。
  - **project scope**（最重）：memory 与 project 零关联；要改存储目录（global + projects/{id}）+ 工具签名加 projectId + prompt 链路传 project_id + 迁移。

### 关键文件
- task: `src-tauri/src/chat/todo.rs`, `types.rs`(AgentTodoItem), `src/chat/AgentTodoIndicator.tsx`, `src/chat/Chat.tsx`(onChatTodo), `src/api/tauri.ts`
- memory: `src-tauri/src/chat/memory.rs`, `settings.rs`(ChatMemoryConfig), `mcp/native_registry.rs`, `chat/commands.rs`(chat_memory_prompt_for_request)

