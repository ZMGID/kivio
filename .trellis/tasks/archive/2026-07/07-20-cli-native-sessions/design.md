# Design — pi/kimi 原生会话接入与会话-CLI 绑定

前提：前三个子任务已合入（含验收反馈修复 268d328/d86e355/6289822/254bd72）。

## 1. kimi 迁移 ACP（R2）

`defs/kimi.rs` 重写为 ACP 家族成员（可直接用 `defs/acp.rs` 的 `acp_def` 构造器加数据行，或独立 def——取决于 kimi 是否需要特殊 launch args；`kimi acp` 无额外 flag，优先加进 acp.rs 数据行，删独立文件）：
- launch args `["acp"]`；`StreamFormat::AcpJsonRpc`、`ModelProbeStrategy::Acp`、`SlashStrategy::Acp`、`supports_native_image: true`（ACP image block）。
- fallback_models 保留现校准表；`list_models_args`（provider list --json）删除——ACP 探测成为主路径，kimi 分支的 `parse_models_list` 保留无害可删（删，避免死代码）。
- `--add-dir`：ACP 模式下目录授权走 session/new 的 cwd + Kivio 现有 extra_allowed_dirs 机制不适用（ACP def 无 dir flag）——kimi acp 以 cwd 为工作区，附件目录在 prompt 附件块中说明，与其他 ACP agent 一致。不单独接 --add-dir（launch args 是常量）。
- 迁移后 kimi 自动获得：live registry 持久会话、session/load resume、中途换模型、错误分类重连、stderr 排空。

## 2. pi 原生会话（R1）

pi 走 `PiRpc` 每轮 spawn（保留——pi 的 rpc 模式本身是常驻交互，但现实现为每轮一个进程）。最小改动方案：**`--session-id` 绑定**。
- Kivio 为每个 conversation 生成固定 session id：`kivio-<conversation_id>`（pi `--session-id` 语义：不存在则创建，存在则续接——首轮创建、后续轮自动带历史）。
- `build_pi_args` 从 `RuntimeContext` 取 id：复用现有 `resume_session_id`/`new_session_id` 字段。pi def 设 `resumes_session_via_cli: true`，接入 `resolve_agent_resume_context` 现有机制（claude 同款）：首轮 `new_session_id`（生成 `kivio-<uuid>`，落盘 external-agent-sessions），后续轮 `resume_session_id`。
- args：`--session-id <id>`（两种情况同一 flag，pi 语义天然幂等）。
- model-mismatch → fresh 的现有逻辑（session/mod.rs）对 pi 同样生效（换模型 = 新 session id，上下文重置——可接受，pi --model 是启动参数）。

## 3. 废除历史重放（R3）

`compose_external_prompt`：
- 删 `build_transcript` 与 `skip_transcript` 参数——全部 CLI 均有原生会话（claude resume / codex thread / ACP session/load / pi --session-id），任何轮次只发 instructions（首轮）+ 最新消息。
- 结构简化为：首轮（`is_resuming=false`）= instructions block + `# User request` + latest；后续轮（resume/复用）= latest（+ skill/附件块）。
- **resume 失败降级**：`run_persistent_turn` 的 fresh 重连场景不再有全量历史可发——`first_prompt` 与 `reuse_prompt` 收敛为同一个 prompt（含 instructions 的版本用于 fresh connect，让 CLI 至少有系统指令）。上下文丢失时在 UI 提示：fresh 重连成功后往流里发一条 `Raw`/提示行（"⚠️ 会话上下文已重置（原生会话不可恢复）"），不静默。claude/pi 每轮 spawn 型：resume id 失效 → CLI 自己新建 session，同样只丢上下文不报错。
- `prompt.rs` 测试改写：断言 full_prompt 不含历史消息文本；末条唯一性测试保留（现在天然唯一）。

## 4. 会话-CLI 绑定（R3 后半）

**规则**：conversation 一旦有任何消息（`!messages.is_empty()`），`agent_runtime` 的 kind 与 external_agent_id 不可再变。
- **后端防御**：`chat_set_agent_runtime`（external_agents/commands.rs:131）校验——load 后若 messages 非空且 (kind 或 external_agent_id) 与现值不同 → `Err("会话已绑定 <名>，请新建会话切换 CLI")`。空会话可随意切。model/reasoning/sandbox 变更不受限。
- **前端**：`RuntimePicker` 接收 `locked: boolean`（Chat.tsx 传 `currentConversation && messages.length > 0 && usesExternalRuntime`）；locked 时 chip 可点开 popover 但所有切换项 disabled + 顶部提示行"会话已绑定当前 CLI，新建会话可切换"。内置 ↔ 外部同样锁（外部会话不能切回内置）。注意：内置模型会话不锁（多模型/ModelSelector 逻辑不变）——只锁"已产生消息的外部会话"。

## 5. 影响面与回滚

- 触碰：`defs/acp.rs`（+kimi 行）、删/改 `defs/kimi.rs`、`defs/pi.rs`、`registry.rs`（若 kimi def 迁移）、`prompt.rs`（删 transcript）、`run.rs`（prompt 收敛 + 重置提示）、`session/mod.rs`（pi 接入 resume 机制，若需微调）、`external_agents/commands.rs`（绑定校验）、`detection.rs`（删 kimi parse 分支）、前端 `RuntimePicker.tsx`/`Chat.tsx`（locked）。
- 每个 R 独立 commit。R3（废历史）依赖 R1/R2 先落（否则 pi/kimi 失忆）。
- 回归风险最大点：prompt 收敛后 fresh 重连的 instructions 注入——用 loop_tests 之外的 external_agents 单测覆盖（compose 的两种形态断言）。
