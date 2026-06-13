# P3 — Multi-agent / Subagent

> 来源：`archive/2026-06/06-12-refactor-kivio-agent-architecture-based-on-clawspring`（研究文档 05-subagent-multi-agent.md + 架构提案 P3 段）。P0/P1/P2 已交付，本期收官 multi-agent 薄壳。

## 目标

在既有 `run_agent_loop`（宿主无关纯函数）之上加一层"无 UI 运行入口 + 子 agent 管理器 + spawn 工具"，让模型可以派生子 agent 处理子任务，并在父消息的 tool card 内看到子 agent 的实时嵌套进度。复用既有 host/executor 双 trait、generation 取消、segments 协议、静态 native 注册表，**不改循环本体**。

## 验收标准（P3）

1. 模型 spawn 子 agent（`agent` 工具，wait=true 同步）返回结果，父消息 tool card 内可见子 agent 嵌套实时进度（折叠/展开）。
2. `depth ≥ 3` 被拒（软失败：返回错误字符串，不打断父循环）。
3. 父 generation 取消时级联取消子 agent（子 host 的 `is_generation_active` 同时校验父 gen）。
4. 子 agent 工具表**不含 `agent` 自身**（第二道递归闸，depth 之外）。
5. 子 agent 是**纯 worker**，不碰 todo：不暴露任何 todo 工具、不注入 todo 提示、不能读写任何 todo 列表（orchestrator-worker 模型）。任务委派自上而下——父对话用自己的 todo 工具把某条 todo 的 `owner` 设为子 agent 名并标 `in_progress`，子 agent 返回后父标 `completed`。

## 设计要点

- **depth 透传**：`ToolExecutionContext` + `AgentRunConfig` 加 `depth: u8`；`rounds.rs` 构造 ctx 时从 config 注入；`RegistryToolExecutor` 把 depth/run_id/generation/tool_call_id 写进 `NativeToolContext`。
- **静态注册表**：`NativeToolContext` 扩展 `run_id`/`generation`/`depth`/`tool_call_id`；`NativeToolCall` 新增 `SubAgent` 变体（在 workspace 解析前分派，类似 `Conversation`，但携带 app+state+native_ctx）；`agent`/`check_agent_result`/`list_agent_tasks` 三工具进 `NATIVE_TOOLS`（`bypasses_approval=true`，`enabled=multi_agent 设置开关`）。
- **agents 模块**：`AgentDefinition`（内置 4 个：general-purpose/researcher/coder/reviewer）+ 三层加载（内置 → `~/.kivio/agents/*.md` → 项目 `.kivio/agents/*.md`），复用 `skills/parse.rs` 的 `split_frontmatter`/`parse_list_value`。
- **工具过滤真正生效**：`filter_tools_for_agent` 按 `agent_def.tools` 收窄，并恒剔除 `agent` 工具自身。
- **SubAgentHost**：实现 `AgentHost`；stream delta 降采样转发为父 tool record 的 `structured_content.subagentProgress`；`request_tool_approval` 在 depth>0 对 sensitive 工具直接拒绝；`is_generation_active` = 子 gen ∧ 父 gen（级联取消）。
- **会话解耦**：子 agent 用合成 `conversation_id`（`subagent-{uuid}`）做 generation/streaming 隔离；`tool_conversation_id`（= 父真实会话）供 native 文件工具定位父项目工作区。子 agent 是纯 worker，不暴露任何 todo 工具，因此无法读写父会话 todo。
- **SubAgentManager**（挂 `AppState`）：`tasks: HashMap`、`by_name`、`Semaphore(3)` 并发上限。wait=true 为主路径；wait=false 后台 best-effort。
- **前端**：`ToolCallBlock` 解析 `structured_content.subagentProgress` 渲染可折叠嵌套进度；`tauri.ts`/`types.ts` 增类型。

## 非目标（移交后续）

- SendMessage 收件箱 / 后台 agent 多轮对话（research P2 项）。
- worktree / cwd 文件系统隔离（research P2 项，桌面端价值低）。
- 子 agent 完整轨迹落盘为独立 conversation。
- 用户侧 todo 编辑入口（P4）。

## 验证

- `cargo test`：depth 拒绝、filter_tools_for_agent 剔除 agent、agent .md 解析、native 注册表 EXPECTED_ORDER 含新工具、SubAgentHost 级联取消。
- `npm run typecheck` / `lint`。
- 手工冒烟：模型 spawn researcher 子 agent 同步返回；父取消子终止；tool card 嵌套进度折叠/展开；父对话自上而下分配 todo（owner=子 agent 名）并在子返回后更新状态。

## 交付状态（2026-06-13）

- ✅ `cargo test`：390 passed / 0 failed（新增 sub_agent/agents/filter 单测 + native_registry 顺序/集合断言更新）。
- ✅ `npm run typecheck` + `npm run lint` 通过。
- ✅ 五项验收对应实现：①同步 spawn + `chat-subagent` 嵌套实时进度（折叠/展开）②`depth_allows_spawn`（depth≥3 软失败）③`generation_cascade_active`（父取消级联，全退出路径回收子 generation）④`agent` 工具不进子表（registry enabled=false + filter 双闸）⑤子 agent 为纯 worker，不暴露/不注入/不读写任何 todo；委派自上而下由父对话用自己的 todo 工具完成（owner=子 agent 名）。
- ⚠️ 方向修正（2026-06-13）：早期实现曾让子 agent 接父会话 todo 工具 + 注入父 todo 提示由子 agent 反向认领，这与 orchestrator-worker 行业标准相悖且引入并行写竞态。已改为纯 worker 模型：删除 `handle_agent_spawn` 中给子 agent 追加 `todo` 工具与注入 `todo_prompt` 的逻辑（`build_chat_system_prompt` 的 `agent_todo_prompt` 传 `None`），保留 `tool_conversation_id=父会话` 以便子 agent 文件工具解析父项目工作区。父对话的 todo 工具不变。并行写竞态随之消失。
- ✅ ultracode 对抗式 review（9 agents）：确认 2 个真实问题已修（timeout 未回收子 generation、React index key）；1 项（todo 工具未进 builtin 枚举）核实与既有顶层路径一致、非回归，不改共享 helper。
- 入口：设置「工具运行 → 多 Agent（子 Agent）」开关（默认关，opt-in）。
- 仍延后：后台 `wait=false` + `SendMessage` 收件箱、worktree/cwd 隔离（research 05 的 P2 项）。
