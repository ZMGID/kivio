# 外部 CLI 会话文件磁盘级审计报告

- 日期：2026-07-22
- 范围：只读核对（ls/cat/grep/jq/python3），未修改或删除任何文件

---

## 追加：kimi 25 个空会话——代码级根因定位（是不是 bug？）

**结论：这是 Kivio 侧的真实缺陷（探测逻辑），被 kimi CLI「session/new 即落盘」的特性放大。不是纯 kimi CLI 行为，也不是致命 bug——功能正确性无损，属资源泄漏/卫生级问题（中等）。**

### 空会话到底是什么（磁盘取证）
任取一个空会话 `session_999d89b2`，其 `agents/main/wire.jsonl` 仅 4 行：
`type=metadata` → `type=config.update(systemPrompt)` → `type=tools.set_active_tools` → `type=config.update(modelAlias)`。
**没有任何 user/assistant 消息**——即只完成了 `initialize` + `session/new` 握手、写入了系统提示与工具配置，从未跑过一轮对话。25 个空会话全部是这种「握手即落盘、无对话」的探测残渣。

### 谁在发 session/new（代码路径）
kimi = ACP（`AcpJsonRpc` / `SlashStrategy::Acp` / `ModelProbeStrategy::Acp`）。三条路径会发 `initialize`+`session/new`：

1. **真实对话轮**（`run.rs::run_persistent_turn` → `connect_persistent_session`）——复用内存 live actor，或用 `session/load` 断点续接（复用 native_id，不新建目录）；只有 resume 失败/传输失败才 fresh `session/new`。→ 真实轮**不会**产生空会话。已验证：真实会话 5422254d 一个目录内含 3 轮，干净。

2. **模型探测**（`chat_detect_external_agent_models` → `detect_agent_models` → `probe_models` → `detect_acp_models`，acp.rs:306）——cwd 由 `resolve_detection_cwd` 解析为 **`__global__`**（非项目会话）。所以模型探测的空会话落在 `__global__`（kimi __global__ 确实只有 3 个），**不污染会话工作区**。

3. **斜杠命令探测**（`slash.rs::list_external_cli_slash_commands`，`SlashStrategy::Acp` 分支）——**这是污染会话工作区的元凶**。它的 cwd 由 `resolve_slash_cwd` → `resolve_effective_cwd` 解析为**每会话独立的 chat-workspace 目录**（不是 `__global__`）。而且它一次探测发 **两次** `session/new`：
   - slash.rs:180 先调 `detect_single_agent`（detection.rs:201），该函数在 214 行**又跑了一遍 `probe_models`** → acp `session/new` #1（在会话 cwd）；
   - slash.rs:192 再调 `detect_acp_commands` → `session/new` #2（在会话 cwd）。
   → **每次斜杠探测 = 会话工作区里 2 个空 kimi 会话。**

### 为什么多达 25 个：Kivio 的缓存缺陷放大
`list_external_cli_slash_commands` 只在**命令列表非空**时写缓存（slash.rs:239）。当 kimi 未上报任何斜杠命令（或 `detect_acp_commands` 超时 → `unwrap_or_default()` 得空列表）时，代码在 slash.rs:226-237 **提前 return、跳过缓存**。于是缓存永远 miss。
前端 `InputBar.tsx:652` 的 `useEffect` 依赖 `[conversationId, externalAgentName, usesExternalRuntime]`——**每次切换会话 / 切换 agent / 切运行时都会重新触发**斜杠探测。本次验收正是在 grok/pi/kimi 之间反复切换运行时，导致该 effect 频繁触发；因空结果不缓存，每次触发 = 2 个空会话，12 分钟累计约 25 个，与磁盘观测吻合。

### 为什么 grok 同代码路径却只有 1 个（CLI 差异）
grok 走完全相同的 ACP 探测代码，但 `~/.grok/sessions/<conv-cwd>/` 下只有 1 个真实会话目录、无空壳。说明 **grok CLI 对「只 session/new、无对话」的会话不落盘（惰性持久化，首轮才写目录）**，而 **kimi CLI 在 session/new 时即急切写全套会话目录**，且从不 GC。所以同样数量的探测请求，只有 kimi 留下永久空壳。这一层是 kimi CLI 自身特性，Kivio 无法直接改；但发起探测的**次数**由 Kivio 控制。

### 严重性评估
- **功能正确性：无影响。** 真实会话不受污染，缺陷 1 去重通过、多轮上下文完整、续接正常。
- **卫生/可扩展性：中等。** 空会话随使用无界增长（单会话 12 分钟 25 个），污染 kimi 自身会话历史（`session_index.jsonl` 已 35 条）、浪费磁盘、每次探测还多 spawn 一个 kimi 子进程（CPU/延迟）。正是 Paseo 文档警告的「临时探测会话污染 provider 历史」。
- **定性：Kivio 探测逻辑的资源泄漏缺陷**，非 CLI 特性、非数据/正确性 bug。

### 建议修复（仅建议，未改动）
1. **空结果也缓存**（负缓存 + 独立 TTL）：agent 不上报命令时不应每次 useEffect 都重探——这是放大 25 倍的主因，改这一条即可把污染降到每会话 ≤2。
2. **斜杠探测改用 `resolve_detection_cwd`（`__global__`）**，与模型探测对齐：残渣不再散落各会话工作区，缓存全 App 共享。
3. **斜杠探测里用 `detect_availability_single`（只查 binary+auth）替代 `detect_single_agent`**，去掉那次冗余的 `probe_models`/`session/new`（模型探测已在 __global__ 单独做）。
4. （可选）ACP 斜杠发现复用现有 live 会话，而非一次性 `session/new`；或探测后主动 `session/*` 清理。因 kimi 急切落盘，真正的杠杆是「少发 session/new」，即上面 1-3 条。

---

- 目标：核对 Kivio 侧会话映射与各 CLI 本地会话存储是否健康一致，重点验证「缺陷 1（用户消息重复）」的磁盘级终验，以及探测/临时会话污染情况

---

## 检查项 1：枚举 Kivio external-agent-sessions

路径：`~/Library/Application Support/com.zmair.kivio/external-agent-sessions/`

共 **23** 个映射文件：

| 文件 | agent | protocol | native id / sessionId | model |
|---|---|---|---|---|
| live-conv_ebc107d8-… b4 | grok | acp_json_rpc | 019f8776-9f24-7cc2-a019-85b55a80da8b | null |
| live-conv_2e82a5cf-…16 | kimi | acp_json_rpc | session_5422254d-13fe-47f0-84f5-ca914ec2aa39 | null |
| conv_2b61f105-…0b | pi | persistent(--session-id) | 69562691-c11e-4a74-8f6c-636a5cd9b3e5 | null |
| conv_a2a896fb-…9b | pi | persistent | 0a7892c5-8496-48a1-9ed3-871cd53aa156 | null |
| conv_bb7e6eca-…e7 | pi | persistent | e1482035-3789-4aaf-a891-bd29a4494541 | null |
| conv_47ffbac7-…b4 | claude | persistent | 785bb2c3-3688-44ec-85d4-15f18419abad | claude-opus-4-8[1M] |
| 其余 17 个 conv_* | claude | persistent | (各自 uuid) | 多为 null |

- 协议分布：grok/kimi 用 `acp_json_rpc`（LiveSessionHandle 形态，文件名前缀 `live-conv_`）；pi 与 claude 用 persistent（存 `sessionId`，文件名 `conv_`）。与设计一致。
- 今日验收的三个会话全部在册：
  - grok「123」→ `live-conv_ebc107d8`
  - kimi「7」→ `live-conv_2e82a5cf`
  - pi「42」→ 命中 `conv_2b61f105`（另有 a2a896fb、bb7e6eca 两个 pi 会话也含 42，见下）
- **结论：正常。** 映射文件结构完整，字段齐全，三个验收会话均落盘。

---

## 检查项 2：逐个反查 CLI 侧 + 缺陷 1（用户消息去重）磁盘终验

### grok（native 019f8776…）
- 存储：`~/.grok/sessions/<urlencoded-cwd>/<session-id>/`，cwd 精确指向 `chat-workspaces/conv_ebc107d8-…`。
- 会话目录存在，含 `events.jsonl`(120行)/`chat_history.jsonl`(28KB)/`prompt_history.jsonl`/`summary.json` 等。
- 「123」可见：`prompt_history.jsonl` 记录两轮——turn1 用户 `123`，turn2 用户 `我上一条发的是什么？`，各 **一次**。`chat_history.jsonl` 里助手正确回答「你上一条发的是：**123**」→ 跨轮上下文保持正常。
- 去重：用户 query（`<user_query>123`）在会话中仅出现一次；`123` 字符串出现 4 次全部是助手回显/引用，非用户消息重复。
- **结论：正常，用户消息各一次。**

### kimi（native session_5422254d…）
- 存储：`~/.kimi-code/sessions/wd_conv_2e82a5cf-…/session_5422254d-…/`（注意实际在 `~/.kimi-code`，非 `~/.kimi`；`~/.kimi` 不存在）。
- 活跃会话 `state.json`：`workDir` 指向 conv_2e82a5cf 工作区，`lastPrompt="那个数字是多少？"`。
- 内容在 `agents/main/wire.jsonl`（29 行）。三条用户消息，顺序：
  1. 指令+记忆块（含「记住这个数字：7，只回复OK」）
  2. `我让你记的数字是多少？`
  3. `那个数字是多少？`
  每条 **恰好一次**。「7」在助手/工具回显里多次出现，用户消息未重复。
- **结论：正常，用户消息各一次，多轮上下文保持。**

### pi（三个会话）
- 存储：`~/.pi/agent/sessions/<slug-cwd>/<ISO时间戳>_<sessionId>.jsonl`，每会话单文件，1:1 命中三个 sessionId（见检查项 4）。
- `conv_2b61f105`（9 行，3 用户轮，42×4）用户消息顺序：
  1. 指令块（记住 42）→ 助手 thinking 确认记住 42
  2. `我让你记的数字是多少？` → 助手 recall
  3. `我让你记的数字是多少？` → 助手 recall
  - turn2 与 turn3 文本相同，但两者各自带独立的助手 thinking 回复、交替排列（USER→ASST→USER→ASST→USER→ASST），是「两轮 + 重启后第三轮再次追问同一问题」的真实多轮，**不是** 同一轮消息被写两次的重复缺陷。
- `conv_a2a896fb`（5 行，1 用户轮）、`conv_bb7e6eca`（9 行，3 用户轮）结构同样健康。
- **结论：正常。** 无同轮重复；turn2/3 文本雷同系用户主动重复提问所致，磁盘上每一轮用户消息只写一次。

### 检查项 2 总体结论
**缺陷 1 磁盘级终验通过**：grok/kimi/pi 三侧会话文件中，每条用户消息均只出现一次，无重复写入。

---

## 检查项 3：反向孤儿检查（探测/垃圾会话堆积）

### 🔴 kimi —— 严重污染（本次新增）
- 单个会话工作区 `wd_conv_2e82a5cf-…` 下有 **26 个 session 目录**，其中只有 `session_5422254d`（映射的活跃会话，3 用户轮）有真实内容；**其余 25 个全部为空壳**：`users=0`、无 `lastPrompt`、`title="New Session"`、`wire.jsonl` 均为同一 26723 字节的纯系统提示基线。
- 25 个空会话的 mtime 密集分布在 10:29–10:41（即这一场对话的 12 分钟内），说明每次连接/探测/新建都调用了 ACP `session/new` 却从未复用，也无清理。
- `~/.kimi-code/sessions/wd___global___…` 另有 3 个（模型探测残留）；`session_index.jsonl` 已累积 35 行。
- 这正是 Paseo 文档警告的「临时探测会话污染 provider 历史」问题，在 kimi 侧已实际发生且相当密集（一场对话 = 25 个空会话）。

### 🟡 cursor —— 严重堆积（历史遗留，非本次）
- `~/.cursor/acp-sessions/` 有 **4065 个** 会话目录（16MB），每个仅含一个 76 字节的 `meta.json`（`{schemaVersion, cwd}`），是空探测/注册残留。
- 最新活动为 Jul 20 22:00，今日（Jul 22）仅 1 个 → 与本次 grok/pi/kimi 验收无关，但属同类 ACP 探测污染，已长期累积失控。

### 🟡 grok —— 中度（可接受）
- 每个 cwd 仅 1 个会话目录并被复用（`conv_ebc107d8` 下就一个 019f8776，即映射的那个）→ **不随轮次膨胀**，明显优于 kimi。
- `__global__`（模型探测）下累积 15 个会话（Jul 20 + Jul 22），每个是 2 行的小真实会话（探测 query）。
- 另有探测残留 cwd 目录：`%2F`、`%2Ftmp%2Fgrok-test`、`%2Fprivate%2Ftmp%2Fgrok-test`、`%2Ftmp%2Facp-probe-test`、`%2FUsers%2Fzmair`、`/var/folders/...T/`。总工作区目录 25 个，规模可控。

### ✅ pi —— 干净
- 每会话单 jsonl，无空壳堆积。工作区按 cwd 分目录，均对应真实对话或历史项目。

---

## 检查项 4：pi --session-id 落盘形态

- 三个 pi 映射 sessionId 与磁盘文件名内嵌 uuid **完全对上**：
  - `69562691…` → `2026-07-22T02-08-59-602Z_69562691-….jsonl`（conv_2b61f105 工作区）
  - `0a7892c5…` → `2026-07-22T02-00-11-634Z_0a7892c5-….jsonl`（conv_a2a896fb）
  - `e1482035…` → `2026-07-22T02-24-04-981Z_e1482035-….jsonl`（conv_bb7e6eca）
- 文件名 = ISO 时间戳 + Kivio 映射的 uuid，落盘时间为今日（2026-07-22 UTC，对应本地上午）。
- 上下文完整：conv_2b61f105 单文件内含「记住 42 → 追问 → 重启后再追问」三轮，全部在同一会话文件里（未因重启分裂）。
- **结论：正常。** pi 的 `--session-id` 复用机制在磁盘上验证成立。

---

## 检查项 5：chat-workspaces 卫生

- `~/Library/Application Support/com.zmair.kivio/chat-workspaces/` 下 **61 个 `conv_*` 目录** + 1 个 `__global__`。
- `__global__` 存在且为空（模型探测用的 cwd，未被塞入异常内容）→ 正常。
- 目录数与历史对话量相称，无异常残留。
- **结论：正常。**

---

## 问题清单（按严重级别，仅报告不修）

1. **🔴 严重｜kimi ACP 空会话污染（本次复现）**
   单场对话在 `~/.kimi-code/sessions/wd_conv_2e82a5cf-…/` 下生成 26 个 session 目录，25 个为纯系统提示的空壳。疑因每次连接/探测调用 `session/new` 而不复用活跃 session、且无清理。会随使用持续膨胀，污染 kimi 侧会话历史与磁盘。建议后续排查 kimi live handle 是否在重连/探测时错误新建会话，并加清理。

2. **🟡 中｜cursor acp-sessions 历史堆积 4065 个**
   每个仅一个 76B `meta.json` 空壳，16MB。非本次产生（最新 Jul 20），但属同类 ACP 探测污染，已长期失控。建议评估 cursor 会话生命周期/GC。

3. **🟢 低｜grok 探测残留**
   `__global__` 15 个探测会话 + 若干 `/tmp/grok-test`、`/private/tmp/grok-test`、`acp-probe-test`、`%2F`、`%2FUsers%2Fzmair` 等测试期 cwd 目录残留。规模可控（真实对话按 cwd 复用单会话，不膨胀），可择机清理测试残渣。

4. **ℹ️ 提示｜pi turn2/3 文本相同**
   conv_2b61f105 中两轮用户消息文本一致，经核实为用户主动重复提问（各带独立助手回复、交替排列），非重复写入缺陷，无需处理。

## 各检查项一行结论

1. 枚举映射：✅ 23 个映射完整，grok/kimi/pi 三个验收会话在册，字段齐全。
2. 反查 CLI + 去重：✅ grok/kimi/pi 会话文件均存在、含今日对话，**每条用户消息只出现一次**（缺陷 1 磁盘终验通过）。
3. 孤儿检查：🔴 kimi 单场对话 25 个空会话污染；🟡 cursor 历史 4065 空壳；🟡 grok 探测残留可控；pi 干净。
4. pi 落盘：✅ 三个 sessionId 与文件名 uuid 精确对应，多轮（含重启后第三轮）同一会话文件、上下文完整。
5. chat-workspaces：✅ 61 conv + __global__（存在且为空），卫生正常。
