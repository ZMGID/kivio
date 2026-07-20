# Implement — pi/kimi 原生会话接入与会话-CLI 绑定

前置：前三子任务已合入。阅读：本任务 prd.md/design.md；父任务 research/paseo-reference.md D 节。

## 执行清单（按序）

### Step 1 — kimi 迁 ACP（R2）
- [ ] `defs/acp.rs` 加 kimi 数据行（bin `kimi`，launch `["acp"]`，fallback 沿用现表）；删 `defs/kimi.rs` + registry 引用更新；`detection.rs` 删 kimi parse 分支。
- [ ] 单测：kimi def 走 AcpJsonRpc/Acp 探测断言（仿 acp_defs_build_expected_launch_args）。
- [ ] 本机冒烟：`kimi acp` 手动 initialize+session/new 验证协议成立（不发 AI prompt）。
- 回滚点：commit。

### Step 2 — pi 原生会话（R1）
- [ ] `defs/pi.rs`：`resumes_session_via_cli: true`；`build_pi_args` 读 ctx 的 resume/new session id → `--session-id <id>`。
- [ ] 确认 `resolve_agent_resume_context` 生成的 id 形态适配 pi（`--session-id` 接受任意字符串；若现机制生成 uuid 直接可用）。
- [ ] 单测：首轮带 new id、次轮带同 id 的 args 断言。
- [ ] 本机冒烟：`pi --session-id kivio-test-<rand> -p "记住数字 42" --mode json` 两轮验证续接（允许一次真实小调用，或人工验证后跳过）。
- 回滚点：commit。

### Step 3 — 废历史重放（R3）
- [ ] `prompt.rs`：删 build_transcript/skip_transcript；compose 收敛为 首轮=instructions+latest / 后续=latest。
- [ ] `run.rs`：first_prompt/reuse_prompt 收敛；fresh 重连成功后发上下文重置提示行。
- [ ] 测试改写：不含历史断言 + 两形态 compose 断言；删过时测试。
- 回滚点：commit。

### Step 4 — 会话-CLI 绑定（R3 后半）
- [ ] `chat_set_agent_runtime` 后端校验（消息非空禁切 kind/agent_id，放行 model/reasoning/sandbox）。
- [ ] 前端 locked：RuntimePicker disabled 项 + 提示行；Chat.tsx 传参。
- [ ] 单测：后端校验三组（空会话可切/非空禁切/同 agent 改 model 放行）；vitest locked 渲染。
- 回滚点：commit。

### Step 5 — 全量回归 + 真机
- [ ] `cargo test --lib` + `npm run lint && npm run typecheck && npm test`。
- [ ] 真机（主会话陪跑）：pi 三轮记忆续接、杀 App 重开续接；kimi ACP 多轮 + 模型列表；grok/claude 第 2 轮 prompt 不含历史（dev 日志确认）；绑定锁 UI。

## 审查门（主会话执行）
1. Diff review：UnifiedAgentEvent/事件 payload 零改动；绑定校验错误文案审校。
2. prd.md 全部 Acceptance 项过一遍。
3. trellis-check 复检 → 归档 → 更新 spec external-cli-agents.md（历史重放条款改为"只发最新消息"）。
