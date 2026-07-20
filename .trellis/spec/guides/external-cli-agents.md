# External CLI Agents — 执行契约与约定

> 来源：任务 07-20-external-cli-overhaul（三个子任务 ce76f60 / 4214956 / 3487e05 的审计与修复）。
> 适用：`src-tauri/src/external_agents/**` 及其前端对接面（`src/chat/RuntimePicker.tsx`、`src/chat/api.ts`）。

## 消息链路（prompt.rs / session/acp.rs）

1. **末条 user 消息单一事实源**：`compose_external_prompt` 中 `latest_user_message` 由 `# User request` 尾部唯一承载；`build_transcript` 必须跳过最后一条 user 消息（按 `rposition` 索引，不按文本匹配）。任何改动不得让同一条用户消息在 full_prompt 中出现两次。
2. **禁止全局文本前缀去重**：ACP assistant/thought 输出的去重走 `AcpTextAssembler`（按消息边界维护累积游标，`on_boundary` 只置位、`push_chunk` 的 starts_with 决定是否重置）。一次性驱动 `run_acp_session` 与持久驱动 `AcpSession::run_turn` 必须共用 `acp_apply_session_update`——不要再出现两份拷贝。
3. **流 parser 的 per-message 状态**：类似 `text_streamed` 的"已流式"标志必须在新消息开始（message_start / 新 message id）时复位，不能是整轮全局 bool。

## 会话生命周期（session/*、run.rs、errors.rs）

4. **持久会话必须排空 stderr**：任何 `Stdio::piped()` 的长活子进程都要 `spawn_stderr_tail`（环形 8KB），close/错误路径 join 取尾部。不排空会因管道写满挂死子进程。
5. **错误出口统一走 `errors::classify`**：气泡主文案 = 分类后的可操作中文（Auth 附 per-agent 登录命令表），原始错误 + 退出码 + stderr 尾部折叠进 `<details>`。新增错误路径不得把裸协议错误串直接落气泡。"401" 等数字判据必须 token 边界匹配。
6. **重试策略是纯函数**：`persistent_failure_action` —— cancelled 保留 handle；Auth 永不自动重试；transient 失败 fresh 重连恰好一次；NEEDS_RECONNECT（launch-flag 配置变更）重连恰好一次。改动重连逻辑先改这个纯函数和它的单测。
7. **handshake 错误带阶段前缀**（`spawn:`/`initialize:`/`session-new:`），超时常量 30s 起步，集中在文件顶部。
8. **中途换模型**：ACP 会话轮前 `session/set_config_option`/`set_model`（best-effort ack）；无对应 config 项的（reasoning 为启动 flag，如 grok）走 NEEDS_RECONNECT 重连。UI 所见必须与会话实际配置一致。

## 检测与模型探测（detection.rs、commands.rs、state.rs）

9. **回复路径零探测**：`run_external_cli_reply` 前置只允许 `resolve_binary`（毫秒级）。version/auth/模型探测只属于列表阶段和懒查命令。任何人不得把 `detect_single_agent`/`probe_models` 加回回复路径（audit N2：曾造成每轮 10-25s 延迟）。
10. **流式 reader 对非 JSON 行一律 `continue`**：任何逐行读子进程 stdout 的解析（探测/会话/命令发现）遇到 banner/日志行只跳过，绝不放弃整条流（audit 缺陷 3 的教训；detect_acp_models 曾用 `?` 硬退）。
11. **探测结果必须带来源**：`chat_detect_external_agent_models` 返回 `source: probed|fallback` + `probeError`；fallback 走 30s 负缓存（probed 300s），force 绕过。前端降级必须可见（角标 + 重试），禁止静默降级到静态表。
12. **defs 静态表只是 fallback**：`fallback_models` 首项恒为 `default`（前端 `agent.models[0]` 依赖此契约）；运行时探测到的才是模型事实源。给 CLI 传 flag 前先 `--help` 核实语义（audit N5：pi 曾把目录塞进 `--append-system-prompt`）。

## 测试约定

13. external_agents 的行为修复必须带可红→绿的单测（本轮新增 ~40 组均遵守）；持久路径优先抽纯函数测（assembler / classify / failure_action / build_*_params），live 测试一律 `#[ignore]` 门控。
