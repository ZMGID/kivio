# External CLI Agents — 执行契约与约定

> 来源：任务 07-20-external-cli-overhaul（子任务 ce76f60 / 4214956 / 3487e05 / 3456997 的审计与修复）。
> 适用：`src-tauri/src/external_agents/**` 及其前端对接面（`src/chat/RuntimePicker.tsx`、`src/chat/api.ts`、`src/chat/Chat.tsx` 的运行时切换）。

## 消息链路（prompt.rs / session/acp.rs）

1. **只发最新消息，历史归 CLI 原生会话**（3456997 起）：`compose_external_prompt` 不重放 transcript——首轮 = instructions + 最新消息，resume 轮 = 仅最新消息。全部 9 个 CLI 均有原生会话（claude `--resume` / codex thread / ACP `session/load` 含 kimi / pi `--session-id`）。禁止任何形式的历史重放回归；fresh 重连丢上下文时必须发可见提示（TextDelta blockquote，cancelled 不发）。
2. **禁止全局文本前缀去重**：ACP assistant/thought 输出的去重走 `AcpTextAssembler`（按消息边界维护累积游标，`on_boundary` 只置位、`push_chunk` 的 starts_with 决定是否重置）。一次性驱动 `run_acp_session` 与持久驱动 `AcpSession::run_turn` 必须共用 `acp_apply_session_update`——不要再出现两份拷贝。
3. **流 parser 的 per-message 状态**：类似 `text_streamed` 的"已流式"标志必须在新消息开始（message_start / 新 message id）时复位，不能是整轮全局 bool。
3b. **会话-CLI 绑定**：有消息的外部会话禁切 kind/external_agent_id（后端 `check_runtime_switch_allowed` 纯函数 + 前端 locked）；model/reasoning/sandbox 放行。前端任何"回写运行时"的路径（如 draft 落地）必须与后端放行条件一致——只对空会话生效，否则被校验拒绝卡死发送。

## 会话生命周期（session/*、run.rs、errors.rs）

4. **持久会话必须排空 stderr**：任何 `Stdio::piped()` 的长活子进程都要 `spawn_stderr_tail`（环形 8KB），close/错误路径 join 取尾部。不排空会因管道写满挂死子进程。
5. **错误出口统一走 `errors::classify`**：气泡主文案 = 分类后的可操作中文（Auth 附 per-agent 登录命令表），原始错误 + 退出码 + stderr 尾部折叠进 `<details>`。新增错误路径不得把裸协议错误串直接落气泡。"401" 等数字判据必须 token 边界匹配。
6. **重试策略是纯函数**：`persistent_failure_action` —— cancelled 保留 handle；Auth 永不自动重试；transient 失败 fresh 重连恰好一次；NEEDS_RECONNECT（launch-flag 配置变更）重连恰好一次。改动重连逻辑先改这个纯函数和它的单测。
7. **handshake 错误带阶段前缀**（`spawn:`/`initialize:`/`session-new:`），超时常量 30s 起步，集中在文件顶部。
8. **中途换模型**：ACP 会话轮前 `session/set_config_option`/`set_model`（best-effort ack）；无对应 config 项的（reasoning 为启动 flag，如 grok）走 NEEDS_RECONNECT 重连。UI 所见必须与会话实际配置一致。
8b. **pi 轮次收尾（07-22 真机验收修复）**：`agent_end` 后进入 3s 宽限排空（到点主动 break，不无限等 EOF——pi 带 `--session-id` 收尾落盘时可能不因 stdin EOF 退出）；drain 返回 Ok 即 `start_kill` 子进程——pi 的会话落盘发生在每条 `message_end` 时（同步 `appendFileSync`），严格先于 `agent_end` 上线，kill 不丢会话、不影响下轮 resume。Unix 下信号退出 `status.code()=None`，不触发出口「非零退出+stderr」规则；**已知边界**：Windows `TerminateProcess` 退出码恒为 1，若该轮 pi 有任何 stderr 输出（如 node 弃用警告）仍可能被误判为 error——Windows 真机复现后需改为「协议层完成标志豁免非零退出规则」而非依赖退出码形态。

## 检测与模型探测（detection.rs、commands.rs、state.rs）

9. **回复路径零探测**：`run_external_cli_reply` 前置只允许 `resolve_binary`（毫秒级）。version/auth/模型探测只属于列表阶段和懒查命令。任何人不得把 `detect_single_agent`/`probe_models` 加回回复路径（audit N2：曾造成每轮 10-25s 延迟）。
10. **流式 reader 对非 JSON 行一律 `continue`**：任何逐行读子进程 stdout 的解析（探测/会话/命令发现）遇到 banner/日志行只跳过，绝不放弃整条流（audit 缺陷 3 的教训；detect_acp_models 曾用 `?` 硬退）。
11. **探测结果必须带来源**：`chat_detect_external_agent_models` 返回 `source: probed|fallback` + `probeError`；fallback 走 30s 负缓存（probed 300s），force 绕过。前端降级必须可见（角标 + 重试），禁止静默降级到静态表。**负缓存同样适用于斜杠命令探测**（07-22 probe-hygiene）：空列表按 `SLASH_COMMANDS_EMPTY_CACHE_TTL`（30s）负缓存、非空 300s（get 侧按 entry 空/非空裁定 TTL）——空结果不写缓存会导致切会话/切 agent 每次重探（kimi 每次探测落一个空壳会话）。
11b. **探测 cwd 统一走 `resolve_detection_cwd`**（非项目会话 = `chat-workspaces/__global__`，绑定项目的会话 = 项目根）：模型探测、斜杠命令探测，以及**所有读写探测缓存的 key**（`slash::cache_key(agent_id, detection_cwd)`）必须用同一 cwd——run.rs 运行时学到的斜杠列表写入、context.rs 的模型缓存读取都曾因 key 用执行 cwd 而与探测 key 分叉（恒 miss / 永远覆盖不了）。执行 cwd（`resolve_effective_cwd`，每会话独立 workspace）只用于真正跑轮次，不得用于缓存 key。斜杠探测本身也用 `detect_availability_single`（不连带 `probe_models`），探测残渣（CLI 落盘的空壳会话）只进 `__global__`。
12. **defs 静态表只是 fallback**：`fallback_models` 首项恒为 `default`（前端 `agent.models[0]` 依赖此契约）；运行时探测到的才是模型事实源。给 CLI 传 flag 前先 `--help` 核实语义（audit N5：pi 曾把目录塞进 `--append-system-prompt`）。**CLI 当前配置（current_model/current_reasoning）的本地配置读取**（codex `~/.codex/config.toml`、pi `~/.pi/agent/settings.json`、kimi `~/.kimi-code/config.toml`）只允许挂在 `detect_agent_models`（懒查模型探测路径，probe 成功分支），禁止进回复热路径；配置缺失/解析失败一律 `None`（前端显示「自动」），解析器必须有非法输入不 panic 的单测。

## 测试约定

13. external_agents 的行为修复必须带可红→绿的单测（本轮新增 ~40 组均遵守）；持久路径优先抽纯函数测（assembler / classify / failure_action / build_*_params），live 测试一律 `#[ignore]` 门控。
