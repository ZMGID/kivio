# Kivio 外部 CLI Agent 子系统 — 全面缺陷审计

- **范围**：`src-tauri/src/external_agents/**`（run / prompt / detection / session/* / defs/* / stream/* / spawn / commands / types）+ 前端对接面（`src/chat/RuntimePicker.tsx`、`src/chat/api.ts`、`src/settings/ExternalAgentsSettings.tsx`）
- **日期**：2026-07-20
- **方法**：逐文件通读 + 调用链回溯（`reply.rs` → `run_external_cli_reply` → 各 session runner）；本机已安装 CLI 版本抽查（见 F）。未发起任何真实 AI 会话。
- **本机已装 CLI（版本）**：claude 2.1.207 / codex-cli 0.144.5 / cursor-agent 2026.06.15 / opencode 1.16.2 / grok 0.2.106 / pi 0.79.7 / kimi 0.27.0；gemini、hermes 未安装。

严重级别定义：**阻断**=功能不可用/挂死/进程泄漏；**严重**=用户可见的错误行为、明显性能损失、数据错误；**一般**=退化、边角、可维护性。

---

## 已确认的四个缺陷（边界核实）

### 缺陷 1 — 用户消息发两遍（transcript + latest 重复）
- 位置：`external_agents/prompt.rs:51`（`build_transcript`）+ `prompt.rs:65`（追加 `latest_user_message`）；触发方 `run.rs:153-163`（`skip_transcript = resume_ctx.is_resuming`）。
- 边界核实：调用方 `chat/commands/reply.rs:95-102` 取 `conversation.messages` 里**最后一条 user** 作为 `latest_user_text` 传入。此时该消息**已在** `conversation.messages` 中。当 `skip_transcript == false`（非 resume）时，`build_transcript` 遍历全部消息（含最后一条 user），随后 `prompt.rs:65` 再把同一条追加 → 末尾出现两遍。
- 受影响面（比原描述更广）：
  - **所有 agent 的第 1 轮**（fresh，`is_resuming=false`）：transcript 此时就只有那一条 user，紧接又追加 → 首轮必重复。
  - **claude**（`resumes_session_via_cli=true`）：仅首轮重复，后续轮 `is_resuming=true` → `skip_transcript=true` 干净。
  - **持久协议 codex/ACP（cursor/gemini/opencode/hermes/grok）**：`resumes_session_via_cli=false` → `resolve_agent_resume_context` 早退，`is_resuming` 恒 false，`composed.full_prompt` 每轮都带 transcript+dup；但 `run_persistent_turn` **复用**时只发 `reuse_prompt`（=latest，干净），仅 **fresh connect（首轮/掉线重连）** 用 `first_prompt` → 重复。
  - **pi / kimi**（非持久、非 resume）：**每一轮**都走 `build_transcript` 全量回放 + 末条重复（见 N9 的放大效应）。
- 严重级别：**严重**。影响面：模型看到重复末条、token 浪费、可能被诱导重复作答。
- 修复方向：`build_transcript` 排除最后一条 user，或 compose 时不再单独追加 `latest_user_message`（二选一，勿两处都保留）。

### 缺陷 2 — ACP `agent_message_chunk` 跨消息边界去重失效 → 回复重复显示
- 位置：`session/acp.rs:695`（`run_acp_session`）与 `acp.rs:1132`（`acp_apply_turn_update`，持久路径）。两处逻辑相同：`let delta = if text.starts_with(&emitted_text) { text[emitted_text.len()..] } else { text }`。
- 机理核实：`emitted_text` 是**整轮全局**累积。对「累积式」ACP agent（每条 message 的 chunk 是从**本消息开头**累积，而非整轮累积），一旦经历工具调用产生**新消息**，新消息第一块无法命中旧的全局前缀 → 走 `else` 原样发出；第二块相对新消息是累积（含第一块内容），但相对**全局** `emitted_text` 仍不匹配 → 再次整块发出 → 第一块内容被重复。示例：msg1="Hello"，工具后 msg2 累积块 "Answer" 然后 "Answer is 42" → 输出 "…HelloAnswer" + "Answer is 42" = 重复 "Answer"。
- 严重级别：**严重**。影响面：cursor/gemini/opencode/hermes/grok 等所有 ACP agent，多轮工具调用后正文重复。
- 修复方向：按 message 边界重置 `emitted_text`（监听 message-start / 消息 id 变化），或对每条 message 单独维护累积游标；纯增量 agent 与累积 agent 需区分处理。

### 缺陷 3 — `detect_acp_models` 遇一行非 JSON 即整体放弃 → 静默降级
- 位置：`session/acp.rs:217` `let value: Value = serde_json::from_str(line.trim()).ok()?;`。`?` 在**任意**非 JSON 行上直接 `return None`。
- 对比：`detect_acp_commands`（`acp.rs:349`）、`run_acp_session`（`acp.rs:1030`）、`codex_app_server.rs:302`、`pi_rpc.rs:62` 都用 `match … Err(_) => continue`。只有 `detect_acp_models` 用 `?` 硬退。
- 严重级别：**严重**。影响面：任何在 stdout 打印过一行 banner/日志的 ACP CLI，模型探测返回 None → `probe_models` 回退 `fallback_models` → 下拉框永远只有 Default（模型选择形同虚设）。
- 修复方向：改为 `Err(_) => continue`，与其他 reader 一致。

### 缺陷 4 — 错误裸奔进聊天气泡，无分类无重试
- 位置：`run.rs:390-431`。`read_result` 为 Err 时（`err != "cancelled"` 且 content/raw 为空）→ `raw_output = format!("{} 读取输出失败：{}", def.name, err)`（`run.rs:398`）。
- 具体裸奔错误串（均原样落入气泡）：`"ACP handshake timeout"`（`acp.rs:1161`，connect 15s/20s）、`"ACP agent exited during handshake"`（`acp.rs:1165`）、`"codex app-server handshake timeout"`（`codex_app_server.rs:641`）、ACP `rpc_error_message`（如 "Authentication required"，`acp.rs:720`）、`"外部 CLI 会话已结束，请重试"`（`run.rs:662`）。
- 严重级别：**严重**。影响面：鉴权失败/握手超时无法区分，用户无从修复；无自动重试/重连。
- 修复方向：在 `run.rs` 出口按 err 前缀分类（auth / timeout / exited / protocol），映射为可操作的中文提示；对握手超时/进程死掉做一次 fresh 重连重试。

---

## A. 会话路径审计（状态机 / 超时 / 取消 / 进程 / stderr / 事件）

### A1【严重 · 新发现 N1】持久会话 stderr 从不排空 → 管道写满可致子进程挂死
- 位置：`AcpSession::connect`（`acp.rs:869-878`）与 `CodexAppServerSession::connect`（`codex_app_server.rs:423-432`）均以 `Stdio::piped()` 开 stderr，但**全程不 take、不读**。非持久路径有 `drain_stderr`（`run.rs:283`），持久路径 `spawned_opt=None`（`run.rs:237`）故无任何 stderr 排空。
- 影响：长活的 codex/ACP 进程若向 stderr 写超过管道缓冲（~64KB，日志/告警常见），write 阻塞 → 整个 CLI 卡死 → 会话永久 hang（外层无墙钟守卫，仅靠用户取消）。
- 修复方向：connect 后 `child.stderr.take()` 起一个 drain 任务（可复用 `drain_stderr`），Close 时结束。

### A2【严重 · 新发现 N3】持久 ACP 会话模型/推理变更被静默忽略
- 位置：`AcpSession::run_turn`（`acp.rs:974`）签名只有 `prompt/images/events/control`，**不接收 model/reasoning**；模型只在 `connect`（`acp.rs:927`）设一次。对比 `CodexAppServerSession::run_turn`（`codex_app_server.rs:497`）每轮应用 `model`/`effort`。
- `run_persistent_turn` 复用分支（`run.rs:598-599`）拿到既有 control 后照样把 `model/reasoning` 塞进 `RunTurn`（`run.rs:648-656`），但 ACP actor（`acp.rs:1226-1234`）**丢弃**这两个字段。
- 影响：会话中途切模型/推理档，对 codex 生效、对所有 ACP agent（cursor/gemini/opencode/hermes/grok）无效，直到会话被 idle(600s)/LRU/进程死回收后重连才生效。UI 显示已切换，实际未切 → 数据错觉。grok 的 reasoning 是启动 flag（`defs/grok.rs:45`），持久会话同样无法中途改。
- 修复方向：ACP `run_turn` 支持每轮 `session/set_model`/`set_config_option`；或在 model/reasoning 变更时强制重连（类似 CLI-resume 的 model-mismatch 逻辑）。

### A3【一般】`run_acp_session` 一次性路径：session/prompt 完成后靠 EOF 收尾，但退出条件依赖 finished
- 位置：`acp.rs:831-840`，`prompt_request_id` 命中后置 `finished=true` 并 `stdin.shutdown()`；随后循环靠 `Ok(Ok(None))`（EOF）break（`acp.rs:634-638`，此时 finished=true 才正常 break）。若 agent 在 prompt 响应后不关闭 stdout 且不再有行，loop 会以 200ms 空转直到 `cancel_check` 或进程退出——一次性路径**无墙钟超时**（仅握手 `acp_read_until_id` 有超时，正式 prompt 阶段无）。
- 影响：一次性 ACP agent 若响应后挂住 stdout，read 循环空转，`spawned.child.wait()`（`run.rs:381`）随后可无限阻塞。
- 修复方向：prompt 阶段加整体墙钟守卫（测试里 `cursor_acp_smoke` 用 90s timeout 佐证需要外部守卫）。

### A4【一般】`run_persistent_turn` 事件收尾竞态（run.rs:672）
- 位置：`run.rs:668-698`。`done_rx` 就绪后 `while let Ok(e)=events_rx.try_recv()` 清尾。
- 核实：actor 单任务顺序执行——`run_turn` 内所有 `events.send(e).await`（`acp.rs:1064` / `codex_app_server.rs:598`）都在 `done.send(result)`（`acp.rs:1234`）之前完成，mpsc 保序，故 done 到达时残余事件均已入 64 容量缓冲，`try_recv` 能全清。**当前无丢事件**，但该正确性隐式依赖「actor 先发完事件再发 done」，无注释/测试约束，改动 actor 顺序易回归。
- 修复方向：加注释固化不变式，或改为「events 通道关闭（None）后再读 done」。

### A5【一般】取消语义在一次性 vs 持久路径不一致
- 一次性：`cancel_check` 命中 → 直接 `child.start_kill()`（`acp.rs:628`、`codex_app_server.rs:283`、`spawn.rs:218`）。
- 持久：发 `SessionCommand::Cancel`（`run.rs:696`），actor 走协议级 `session/cancel`/`turn/interrupt` **不杀进程**（保活）。但若协议级取消无响应，`run_persistent_turn` 循环仅每 200ms 重发一次判断、`cancel_sent` 后不再升级为强杀 → 卡死的持久会话取消不掉（只能等 idle 回收）。
- 修复方向：Cancel 后设兜底超时，超时升级 Close（强杀 + 清 registry）。

### A6【一般】非持久路径 `child.wait()` 可阻塞在 read_result Err 之后
- 位置：`run.rs:379-385`。read_result 出错（如 cancelled 已 start_kill 尚可），但若错误发生时子进程仍在跑且未被 kill（例如 `Ok(Err(e))` I/O 错误分支 `spawn.rs:224` 直接 return Err 未 kill），`spawned.child.wait()` 需等其自行退出。`kill_on_drop(true)` 只在 spawned 被 drop 时兜底——而这里先 `wait()` 再 drop。
- 修复方向：错误路径统一 `start_kill()` 后再 `wait()`。

---

## B. 检测与缓存

### B1【严重 · 新发现 N2】每轮回复都跑**完整模型探测**（不止可用性）
- 位置：`run.rs:88` `let detected = detect_single_agent(def, &cwd).await;`，随后只用 `detected.available`（`run.rs:89`）。
- 核实：`detect_single_agent`（`detection.rs:99-131`）在 available 时**必调 `probe_models`**（`detection.rs:112`）。即每条外部回复前都会：claude 走 `ClaudeInit`（`detection.rs:222`，超时 10s，真启动 claude 进程写 probe stdin）、ACP agent 走 `detect_acp_models`（`detection.rs:219`，**新起一个 ACP 子进程** initialize+session/new，超时 15s）、codex 走 `codex debug models`、pi 走 `pi --list-models`（20s）。结果被完全丢弃。
- 影响：比任务描述的「几秒」更重——每轮固定几秒到十几秒的无谓延迟 + 额外子进程。且与 `commands.rs` 里精心做的 single-flight + 缓存（`chat_detect_external_agent_models`）完全绕开。
- 修复方向：改用 `detect_availability_single`（`detection.rs:23`，不探模型）或直接 `resolve_binary`；可用性本就与 cwd 无关且有 600s 缓存可复用。

### B2【一般】可用性缓存无「安装后刷新」入口（前端）
- `RuntimePicker`（`RuntimePicker.tsx:39`）与 `PermissionPicker`（`PermissionPicker.tsx:52`）都以 `detectExternalAgents(false)` 读缓存；`force=true` 只在 `settings/ExternalAgentsSettings.tsx:30` 有。`AVAILABILITY_CACHE_TTL=600s`（`detection.rs:19`）。
- 影响：用户新装一个 CLI 后，聊天区 RuntimePicker 最长 10 分钟看不到（除非去设置页手动刷新）。
- 修复方向：RuntimePicker popover 加刷新按钮（force）。

### B3【一般 · 新发现 N8】grok auth 探测与实际会话鉴权口径不一
- `defs/grok.rs:67` `auth_probe_args = Some(&["models"])`，注释称登录时 exit 0（本地缓存），未登录 exit 1。`probe_auth`（`detection.rs:180-198`）以 exit 成功=「ok」。
- 风险：`grok models` 从本地缓存返回，**不代表** ACP prompt 会话在线鉴权仍有效（token 过期后 models 仍可能命中缓存 exit 0）→ auth 显示 ok 但真会话 401，最终以裸错误落入气泡（缺陷 4）。类似地 cursor 用 `status`、codex 用 `login status`、claude 用 `auth status`，各自口径与真会话鉴权不完全等价。
- 修复方向：auth 仅作提示、不作门禁；会话侧 401 需可分类重试（并联缺陷 4）。

### B4【一般】`detect_availability_all` 忽略被 panic 的探测句柄
- `detection.rs:56-60`：`if let Ok(agent)=handle.await`，某 agent 探测 panic 会被静默丢弃，该 agent 从列表消失且无日志。低概率但难排查。

---

## C. prompt / 会话恢复语义

### C1【严重】fresh 持久会话首轮把缺陷 1 的重复 prompt 发给服务端
- `run_persistent_turn`：fresh/无法 resume 时 `prompt = first_prompt`（`run.rs:639`）= `composed.full_prompt`（带 transcript+重复末条）。复用/resume 时 `reuse_prompt`（干净）。故缺陷 1 与持久路径的交互结论：**首轮或掉线重连时**双重复；稳态复用不复现。修复缺陷 1 即可消除。

### C2【严重 · 见 A2/N3】被复用的持久会话 model 变更无处理
- `resolve_agent_resume_context`（`session/mod.rs:104-117`）里 model-mismatch→fresh 的逻辑**只对 `resumes_session_via_cli=true`（claude）生效**；对 codex/ACP（false）在 `mod.rs:84` 早退，完全不参与。持久会话的 model 变更改由 live registry 处理，但如 A2 所述 ACP 侧未实现 → 静默无效。
- 另：`session/load`（`acp.rs:904-911`）resume 时若服务端已换 model，客户端无校验。

### C3【一般】会话失效后回退语义基本正确，但有一处清理不对称
- `run_persistent_turn`：control.send 失败（actor 死）→ `remove_external_live_session` + `clear_live_handle`（`run.rs:660-662`），下轮 fresh，合理。
- done 返回非 cancelled 错误 → `remove_external_live_session` + `clear_live_handle`（`run.rs:678-682`）；cancelled 只 remove 不清 handle（保留以便下次 resume），合理。
- 但 `connect_persistent_session` 里 resume 失败会**静默降级 fresh**（`acp.rs:758-767`、`codex_app_server.rs:735-748` 的 `if let Ok(session)`），此时**旧 live handle 未被清**（仍指向失效 native_id），下次仍会先试一次注定失败的 resume。低危但每次多一次超时。
- 修复方向：resume 失败落 fresh 后覆盖/清理 live handle。

### C4【一般】`persist_delivered_session` 对持久 agent 是 no-op（符合预期，但埋了脆弱假设）
- 持久 agent `resumes_via_cli=false` → `resolve_agent_resume_context` 早退（new_session_id=None,is_resuming=false）→ `persist_delivered_session`（`mod.rs:157-169`）两个分支都不写。正确。但该行为依赖「早退分支把 new_session_id 设 None」这一细节，若未来给持久 agent 打开 resumes_via_cli 会与 live handle 双写冲突。加注释即可。

---

## D. 前端对接面

### D1【一般】模型探测错误对用户不可见
- `ExternalModelSelector`（`RuntimePicker.tsx:208-217`）`.catch(() => {/* 保留上次结果，不清空 */})` 静默吞错。叠加缺陷 3（后端返回只有 Default 且 `success:true`），用户永远只看到 Default，且无任何错误/加载态提示，无法判断是「真的只有 Default」还是「探测失败」。
- `api.ts:1438-1451` `detectExternalAgentModels` 直接取 `models`，后端 `success:false` 分支前端未特别处理。
- 修复方向：区分「探测失败/降级」与「真无模型」，UI 给出降级角标 + 重试。

### D2【一般】无手动刷新入口（同 B2）
- RuntimePicker/ExternalModelSelector 均无 force 刷新按钮；只有设置页有。

### D3【一般】默认模型选取依赖 fallback 首项恒为 default
- `RuntimePicker.tsx:72` `agent.models[0]?.id ?? 'default'`。availability 阶段 `models=fallback_models_from_pairs`（`detection.rs:42`），`fallback_models_from_pairs`（`types.rs:198`）保证首项恒为 `default`，故 OK；但这是隐式契约，若某 def 的 fallback 未含 default 会选错。低危。

### D4【一般】错误消息渲染
- 外部回复错误经 `emit_chat_stream_done(..., stream_outcome, content)`（`run.rs:449`）+ `push_assistant_message`（`run.rs:471`）走与内置一致的气泡渲染；即缺陷 4 的裸错误串会作为**普通 assistant 文本**显示，无错误样式/分类/重试按钮。

---

## E. 测试覆盖缺口

- **缺陷 1 未被现有测试暴露**：`prompt.rs` 唯一的 compose 测试 `compose_includes_instructions_and_user_request`（`prompt.rs:134`）用 `empty_conversation()` + `skip_transcript=true`，绕过了 `build_transcript`。**缺**：非空会话 + `skip_transcript=false` 断言「末条不重复」。
- **缺陷 2 无测试**：`acp.rs` 测试只覆盖 `apply_acp_session_update`（tool_call/update）与 `normalize_models`，**无**对 `agent_message_chunk` 去重、尤其跨消息边界的用例。应加：两段 message 中间夹一次 tool_call，断言正文不重复（增量式与累积式两种 chunk 语义各一）。
- **缺陷 3 无测试**：`detect_acp_models` 无「首行非 JSON 仍能解析出模型」的用例。
- **缺陷 4 无测试**：`run.rs` 错误分类/落地无单测（`run.rs` 现有测试仅覆盖 `StreamSegmentTracker`/`push_tool_segment`）。
- **持久路径几乎无非 live 测试**：`run_persistent_turn`、`connect_persistent_session`、stderr 排空（N1）、model 复用变更（N3）均无覆盖；`AcpSession`/`CodexAppServerSession` 仅有 `#[ignore]` live 测试。
- **A2/N3 无测试**：ACP 中途换模型不生效无回归用例。
- **前端**：`ExternalAgentsSettings.test.tsx` 存在；`RuntimePicker`/`ExternalModelSelector` 无错误态/刷新/降级测试。

---

## F. 各 CLI def 数据正确性抽查

本机版本：claude 2.1.207 / codex-cli 0.144.5 / cursor-agent 2026.06.15 / opencode 1.16.2 / grok 0.2.106 / pi 0.79.7 / kimi 0.27.0（gemini、hermes 未装）。`--version` 对全部已装 CLI 正常返回 → `probe_version`（`detection.rs:160`）OK。

### F1【一般 · 新发现 N5】pi 把 allowed-dirs 塞进 `--append-system-prompt`
- `defs/pi.rs:43-48`：`for dir in &ctx.extra_allowed_dirs { args.push("--append-system-prompt"); args.push(dir) }`。`extra_allowed_dirs` 是**目录路径**（skill 扫描路径 + 会话附件目录，见 `run.rs:187-193`），却被当成「系统提示文本」逐个追加。pi 无 allowed-dir 概念，这些裸路径会作为 system prompt 片段污染上下文，且不起授权作用。疑似复制自 claude 的 `--add-dir` 但改错了 flag。
- 修复方向：确认 pi 是否有目录授权 flag；无则不应把目录塞进 system prompt。

### F2【一般】fallback_models 时效
- codex（`defs/codex.rs:5-10`）fallback `gpt-5.3-codex/gpt-5/o3`：codex 走 app-server，实际模型探测靠 `codex debug models`（`list_models_args`），fallback 仅在探测失败时用，风险低但型号偏旧。
- cursor（`defs/acp.rs:74-79`）fallback `sonnet-4/gpt-5`、gemini `gemini-3-pro-preview` 等、hermes `grok-4.3/gpt-5.5` 等、grok `grok-4.5`（`defs/grok.rs:19`）：均为 ACP 探测（受**缺陷 3** 影响，探测一失败就落到这些静态表）。gemini-3-pro-preview / grok-4.x / gpt-5.5 等型号明显是占位/预期型号，需随 CLI 更新校准。
- kimi（`defs/kimi.rs:6-11`）`kimi-k2-turbo-preview/moonshot-v1-*`、pi 只有 default——kimi 无探测（`model_probe=None`、无 `list_models_args`）→ **恒用 fallback**，型号过期风险最高。
- 修复方向：给 ACP 系补充「探测失败可见降级」（D1），并周期校准静态表；kimi 若 CLI 支持列模型应补 `list_models_args`。

### F3【一般】grok def 版本漂移
- `defs/grok.rs:3` 注释「verified against v0.2.103」，本机 v0.2.106。协议参数 `["agent","stdio"]` 探测、`["agent",...,"stdio"]` 启动；小版本差异需回归。grok 走 ACP 模型探测 → 若 stdout 有 banner 会触发缺陷 3。

### F4【观察】codex `model_probe=None` 但 `list_models_args=Some(["debug","models"])`
- codex 走 `probe_models` 的通用分支（`detection.rs:233`）跑 `codex debug models` 并用 `parse_models_list("codex", …)`（`detection.rs:319`）解析 JSON。逻辑自洽。注意该分支 `list_models_timeout_secs=None`→`unwrap_or(5)`（`detection.rs:234`），`codex debug models` 若需网络/鉴权可能 5s 不够 → 偶发落 fallback。

### F5【观察】opencode 双 bin 名
- `defs/acp.rs:143-153` bin `opencode-cli`，fallback `opencode`。本机只有 `opencode`（fallback 命中）。`resolve_binary`（`spawn.rs:54`）按序探测，OK。

---

## 缺陷总表（按严重级别排序）

| # | 级别 | 位置 | 摘要 | 修复方向 |
|---|---|---|---|---|
| N1 | 阻断 | acp.rs:869 / codex_app_server.rs:423 | 持久会话 stderr piped 但从不排空 → 管道写满挂死子进程 | connect 后起 stderr drain 任务 |
| 1 | 严重 | prompt.rs:51/65 (+run.rs:153) | build_transcript 已含末条，又追加 latest → 用户消息发两遍（首轮/pi/kimi/fresh 重连必现） | transcript 排除末条 或 不再单独追加 |
| 2 | 严重 | acp.rs:695 / acp.rs:1132 | agent_message_chunk 全局前缀去重跨消息边界失效 → 正文重复 | 按 message 边界重置 emitted_text |
| 3 | 严重 | acp.rs:217 | detect_acp_models 一行非 JSON 即 `?` 整体放弃 → 永远 Default | 改 `Err(_)=>continue` |
| 4 | 严重 | run.rs:398 (+acp/codex handshake) | 握手超时/鉴权失败等原样进气泡，无分类无重试 | 出口分类映射 + 握手超时重连一次 |
| N2 | 严重 | run.rs:88 (detection.rs:112) | 每轮回复跑完整模型探测（claude 10s / ACP 起进程 15s / pi 20s），结果丢弃 | 改 detect_availability_single 或 resolve_binary |
| N3 | 严重 | acp.rs:974 (run.rs:648) | ACP 持久会话中途换 model/reasoning 被静默忽略（codex 生效、ACP 不生效） | run_turn 支持 set_model 或变更即重连 |
| C1 | 严重 | run.rs:639 | fresh 持久会话首轮/重连把缺陷1的重复 prompt 发出 | 随缺陷1修复 |
| A3 | 一般 | acp.rs:831/634 | 一次性 ACP prompt 阶段无墙钟超时 → 可空转/wait 阻塞 | prompt 阶段加整体超时 |
| A5 | 一般 | run.rs:696 | 持久取消无强杀兜底 → 协议级取消无响应时卡死 | Cancel 超时升级 Close |
| A6 | 一般 | run.rs:381 / spawn.rs:224 | 错误路径先 wait 后 drop，未先 start_kill 可阻塞 | 错误路径统一先 kill 再 wait |
| A4 | 一般 | run.rs:672 | 事件清尾正确但隐式依赖 actor 顺序，易回归 | 加注释/改 None 后读 done |
| N4 | 一般 | acp.rs:228-247 | detect_acp_models 只读 session/new result，忽略异步 session/update 推送的模型 → 部分 agent 恒 Default（叠加缺陷3） | 参照 detect_acp_commands 读通知 |
| N5 | 一般 | defs/pi.rs:44 | allowed-dirs 被塞进 --append-system-prompt（错 flag，路径污染系统提示） | 用正确的目录授权方式 |
| N8/B3 | 一般 | defs/grok.rs:67 (detection.rs:193) | auth 探测（缓存 exit code）≠ 真会话鉴权 → 显示 ok 但会话 401 | auth 仅提示 + 会话侧 401 分类重试 |
| B2/D2 | 一般 | RuntimePicker.tsx:39 | 无安装后刷新入口（600s 缓存）| picker 加 force 刷新 |
| D1 | 一般 | RuntimePicker.tsx:208 | 模型探测错误静默吞掉 → 用户分不清降级/真无模型 | 降级角标 + 重试 |
| C3 | 一般 | acp.rs:758 / codex:735 | resume 失败降级 fresh 后旧 live handle 未清 → 下次多一次注定失败的 resume | 降级后覆盖/清 handle |
| F1(N5) | 一般 | defs/pi.rs | 同 N5 | — |
| F2 | 一般 | defs/*.rs | fallback_models（尤其 kimi 恒用、ACP 系探测失败即用）型号过期 | 周期校准 + 可见降级 |
| F4 | 一般 | detection.rs:234 | codex 通用分支 timeout 默认 5s，`debug models` 需网络时偏短 | 给 codex 设显式超时 |
| B4 | 一般 | detection.rs:56 | 探测 panic 句柄被静默丢弃，agent 消失无日志 | 记录 join 错误 |
| C4 | 一般 | mod.rs:84 | persist 对持久 agent no-op 正确但依赖脆弱早退细节 | 加注释固化 |
| N7 | 一般 | stream/claude.rs:56/206 | text_streamed 单一全局 bool；仅以 assistant 整块交付的后续消息可能被丢（缺陷2同类，被 include_partial_messages 缓解）| 按 message 复位 |
| N9 | 一般 | prompt.rs:73 | pi/kimi 每轮全量回放 transcript + 末条重复；kimi 计入 argv 30KB 上限 | 随缺陷1 + 考虑非 resume agent 的历史裁剪 |

合计：**26 项**（原确认 4 + 新增/关联 22）。其中 **阻断 1、严重 7、一般 18**。

---

## Caveats / 未覆盖

- ACP `agent_message_chunk` 到底是「整轮累积」「按消息累积」还是「纯增量」，因各家 CLI 实现不同、且我未发起真实会话，缺陷 2 的复现属**基于代码逻辑的推断**（若某 agent 为纯整轮累积则不复现，但按消息累积/混合则必现）——建议用一次 live `cursor-agent`/`grok` 会话（带工具调用）实证。
- N1（stderr 挂死）为**理论管道死锁**，触发需 CLI 向 stderr 写超过缓冲；未实测触发。
- codex `debug models` / ACP 模型探测的真实 stdout 是否含非 JSON 行（决定缺陷 3 命中率）未实跑验证（避免真会话/鉴权）。
- `context.rs` / `compact.rs` / `attachments.rs` / `skill_stage.rs` / `slash.rs` / `workspace.rs` 仅粗读，未逐行审计（本次聚焦会话/检测/prompt/恢复主链）。
