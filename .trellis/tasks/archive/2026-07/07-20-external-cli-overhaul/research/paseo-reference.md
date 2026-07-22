# Research: Paseo 如何驱动外部编码 CLI 作为聊天后端（对照 Kivio external_agents 四类缺陷）

- **Query**: 深入研究参考项目 Paseo 如何驱动 claude/codex/cursor/opencode/gemini 等外部 CLI 作为聊天后端，并为 Kivio `src-tauri/src/external_agents/` 的四类已确认缺陷提供成熟做法参考
- **Scope**: internal（参考项目源码，位于 `/Users/zmair/ZM database/cankao/paseo`，TypeScript monorepo）
- **Date**: 2026-07-20
- **参考项目根**: `/Users/zmair/ZM database/cankao/paseo`（下文所有 `packages/...`、`docs/...` 路径均相对该根）

---

## 0. 全局架构与关键结论（先读）

Paseo 是 client-server 架构：**daemon（`packages/server`，Node.js）** 负责 spawn/管理各 CLI 进程并把它们的输出流归一化后经 WebSocket 推给客户端（`packages/app` Expo / `packages/cli` / `packages/desktop` Electron）。核心抽象是 `AgentClient` / `AgentSession` 接口（`packages/server/src/server/agent/agent-sdk-types.ts`），每个 provider 是一个适配器。

对 Kivio 四缺陷最重要的四条 Paseo 设计原则（后文逐条展开）：

1. **只发当前 prompt，绝不重发 transcript。** 历史由 CLI 自己的原生 session 保存，Paseo 通过 resume/loadSession 让 CLI 恢复历史，`startTurn` 只把最新一条 prompt 交给 CLI。→ 直接对应缺陷 1。
2. **去重按「消息/分片 ID」做，不做全局文本前缀去重。** 文档白纸黑字写：`Do not perform global transcript text dedupe`。ACP 只对「用户消息回显」在当前 turn 内做前缀匹配；OpenCode 用 **per-part-ID** 记录已流式的分片，终态快照若该分片 ID 已流过则丢弃。→ 直接对应缺陷 2。
3. **传输层按行容错解析 JSON，坏行只 warn 并跳过，绝不放弃整条流。** ACP / Codex app-server 传输都是逐行 `JSON.parse` + try/catch，banner 行不影响后续 JSON-RPC 响应。模型探测因此不会被 banner 打死。→ 直接对应缺陷 3。
4. **探测失败进入显式 `error` 状态并携带错误消息 + 结构化诊断，而不是静默降级到静态 fallback。** ProviderSnapshotManager 有 `loading/ready/unavailable/error` 四态；探测超时/失败写 `status:"error"` + `error` 字符串给 UI。另有分阶段诊断（spawn/initialize/session-new/cleanup）。→ 直接对应缺陷 4。

---

## A. Provider/Agent 检测与模型目录

### A.1 两种集成模式

`docs/providers.md`（开头）定义两种 provider 集成模式：

- **ACP（Agent Client Protocol，推荐）** — 继承 `ACPAgentClient`（`packages/server/src/server/agent/providers/acp-agent.ts`），基类统一处理进程 spawn、stdio 传输、session 生命周期、streaming、权限、**模型发现**。子类只给配置（command、modes、capabilities），必要时覆写 `isAvailable()` 做鉴权检查。内建 ACP provider：`copilot`（`copilot-acp-agent.ts`）、`cursor`（`cursor-acp-agent.ts`，仅 45 行）、`generic-acp-agent.ts`（用户自定义 `extends: "acp"`）、`kiro`/`trae`。
- **Direct** — 自己实现 `AgentClient`/`AgentSession`。现有 direct provider：`claude`（`providers/claude/agent.ts`）、`codex`（`codex-app-server-agent.ts`）、`opencode`（`opencode-agent.ts`）、`pi`（`providers/pi/agent.ts`）、`omp`（`providers/omp/agent.ts`）。

### A.2 安装/登录态检测：`isAvailable()`

`isAvailable()` 是 provider 自报可用性的唯一入口。分层：先查二进制在 PATH（`checkProviderLaunchAvailable`），再叠加鉴权检查。

Claude 的实现（`providers/claude/agent.ts:1535`）：

```ts
async isAvailable(): Promise<boolean> {
  const launch = await resolveProviderLaunch({
    commandConfig: this.runtimeSettings?.command,
    defaultBinary: "claude",
  });
  const availability = await checkProviderLaunchAvailable(launch);
  return availability.available;
}
```

ACP 基类（`acp-agent.ts:971`）：

```ts
async isAvailable(): Promise<boolean> {
  try { await this.resolveLaunchCommand(); return true; } catch { return false; }
}
```

`docs/providers.md` 的 Gotchas 明确：鉴权模式各不相同 —— 有的查 env（`ANTHROPIC_API_KEY`/`OPENAI_API_KEY`），有的查 OAuth token（`CLAUDE_CODE_OAUTH_TOKEN`），有的查 auth 文件（`~/.codex/auth.json`），Copilot 完全由 CLI 自己处理。子类只需在 `super.isAvailable()` 之上叠加自己的凭据检查。

### A.3 模型目录：`fetchCatalog` 是唯一发现 API

`docs/providers.md`：`fetchCatalog` is the single discovery API for models and modes —— 外部调用方不再单独探测模型/模式。分三种来源：

1. **ACP：运行时探测。** `acp-agent.ts:851` `fetchCatalog()`：spawn 一个**临时探测进程** → `newSession({cwd, mcpServers:[]})` → 从响应里 `deriveModelDefinitionsFromACP` + `deriveModesFromACP` → 关闭探测进程。整个探测用 `withTimeout` 包裹，超时常量 `ACP_CATALOG_TIMEOUT_MS = 60_000`（`acp-agent.ts:261`）。global scope 用 `homedir()` 作 cwd，workspace scope 用具体 cwd。

2. **Claude：静态 manifest。** `providers/claude/model-manifest.ts` 是 Claude 一方模型元数据的唯一来源，model picker 的 thinking 选项、feature gate 都从 manifest 派生（`docs/providers.md`：`update that manifest only`）。

3. **Codex/OpenCode/Pi：各自 SDK/进程上游 API。** `fetchCatalog` 内部可以用一个进程、多次上游调用或静态数据，对外统一。

**模型派生逻辑（`deriveModelDefinitionsFromACP`，`acp-agent.ts:644`）—— 缺陷 3 关键：**

```ts
export function deriveModelDefinitionsFromACP(provider, models, configOptions) {
  const thinkingOptions = deriveSelectorOptions(configOptions, "thought_level");
  ...
  // 优先：ACP session state 的 availableModels 字段
  if (models?.availableModels?.length) {
    return models.availableModels.map((model) => ({ provider, id: model.modelId, label: model.name, ... }));
  }
  // 回退：configOptions 里 category === "model" 的 select 选项
  const modelOptions = deriveSelectorOptions(configOptions, "model");
  return modelOptions.map((option) => ({ provider, id: option.id, label: option.label, ... }));
}
```

要点：Paseo **同时**支持两条 ACP 模型来源（session 的 `availableModels` 字段 **和** `configOptions` 里 `category:"model"` 的 select），任一有值就用；**都没有时返回空数组，而不是塞一个静态 fallback**。空 ≠ 报错（见 A.5）。

### A.4 ProviderSnapshotManager：缓存/失效策略

`packages/server/src/server/agent/provider-snapshot-manager.ts`（1000 行）。

- **按 cwd 分桶缓存**：`private snapshots = new Map<string, Map<AgentProvider, ProviderSnapshotEntry>>()`（`:174`）。另有语义化的全局 scope key `GLOBAL_PROVIDER_SNAPSHOT_KEY = "paseo:global"`（`:41`），用于设置/provider 管理和无 cwd 的请求。
- **四态 entry**：`loading | ready | unavailable | error`（见 `refreshProvider` `:750`）。`ready` 携带 `models/modes/fetchedAt`，`error` 携带 `error` 字符串。
- **单飞（single-flight）**：`providerLoads` map 记录进行中的 load，`loadProvider`（`:708`）里 `if (existingLoad && !options.force) return existingLoad.promise;`；`if (existingEntry && existingEntry.status !== "loading" && !options.force) return;`。
- **无 TTL、冷读才探测**：`docs/providers.md`「Provider Snapshot Refresh Contract」（:61-71）—— 一旦 entry warm，`ready/error/unavailable` 状态就缓存到显式刷新为止。**明令禁止** TTL 重验证、focus 触发刷新、selector 打开刷新、config-reload 刷新。
- **两级 scope 探测**：`FetchCatalogOptions` 是判别联合：`{scope:"global", force}` 或 `{scope:"workspace", cwd, force}`。
- **设置刷新语义**：`refreshSettingsSnapshot`（:223）是用户面的「忘掉所有过期 provider 知识」动作 —— 清空所有 cwd scope 的缓存与在途 load，然后**只**用 `force:true` 立即刷新 global，workspace 快照留到下次 scoped 读时惰性重探。
- **registry 替换不 spawn 进程**：config 变更只更新可见元数据（label/description/enabled 等），要重探得走显式 settings refresh。

超时常量：`DEFAULT_REFRESH_TIMEOUT_MS = 60_000`，`DEFAULT_DIAGNOSTIC_TIMEOUT_MS = 120_000`，可用 env `PASEO_PROVIDER_REFRESH_TIMEOUT_MS` 调大（`:38-59`，注释说明 Copilot 首次 `copilot --acp`、OpenCode 多 MCP server 探测冷启动很慢）。

### A.5 探测失败如何呈现（缺陷 4 直接参考）

`refreshProvider`（`provider-snapshot-manager.ts:750`）核心流程：

```ts
if (!definition.enabled) { setEntry({...base, status: "unavailable", enabled: false}); return; }
const available = await withTimeout(client.isAvailable(), this.refreshTimeoutMs, `Timed out checking ${label} availability ...`);
if (!available) { setEntry({...base, status: "unavailable", enabled: true}); return; }
try {
  const catalog = await withTimeout(definition.fetchCatalog({...catalogOptions, timeoutMs}, client), this.refreshTimeoutMs, `Timed out refreshing ${label} ...`);
  setEntry({...base, status: "ready", models: catalog.models, modes: catalog.modes, fetchedAt: new Date().toISOString()});
} catch (error) {
  setEntry({...base, status: "error", enabled: true, error: toErrorMessage(error)});  // ← 不静默降级，把错误交给 UI
  this.logger.warn({ err: error, provider, cwd }, ...);
}
```

关键对比 Kivio 缺陷 3/4：**探测超时/失败 = `status:"error"` + 具体错误消息**，UI 能区分「探测失败（error）」「未安装/未登录（unavailable）」「探测成功但无模型（ready + 空 models）」三种，而不是一律回退成静态「Default」。

---

## B. 会话与协议驱动

### B.1 各 CLI 用什么协议（`docs/architecture.md` "Agent providers" 表）

| Provider | 包装的东西 | Session 存储 |
|---|---|---|
| Claude | Anthropic Agent SDK | `~/.claude/projects/{cwd}/{session-id}.jsonl` |
| Codex | Codex AppServer（JSON-RPC over stdio） | `~/.codex/sessions/{date}/rollout-*.jsonl` |
| Copilot / Cursor / Generic | ACP | provider 自管 |
| OpenCode | OpenCode server/CLI | provider 自管 |
| Pi / OMP | 本地 `pi --mode rpc` / `omp --mode rpc-ui`（JSONL 子进程 RPC） | provider 自管 |

多协议适配器彼此对等，都实现同一 `AgentClient`/`AgentSession` 接口。

### B.2 进程生命周期：常驻 session + 临时探测进程

- **正式 session：常驻子进程。** `initializeNewSession`（`acp-agent.ts:1378`）里 `spawnProcess()` 拿到 `this.child`/`this.connection`，`newSession` 建会话，进程存活到 session 关闭。
- **探测/元数据查询：临时进程即用即关。** `fetchCatalog`/`listFeatures`/`listImportableSessions` 都是 spawn 探测进程 → 查 → `closeProbe`（`terminateChildProcess`）。`docs/providers.md` 特别强调：草稿元数据查询优先用 `fetchCatalog`/`listCommands`/`listFeatures`，**别创建临时 session**（临时 session 会污染 provider 的 import/history UI）。
- **运行时驻留回收**（`docs/agent-lifecycle.md`）：idle 的 agent 空闲 2 分钟后被回收（释放进程/订阅但保留 Paseo 身份），每 15s 扫一次；`running/initializing/error` 不回收；下次 prompt 时通过 `ensureAgentLoaded()` → resume 恢复。

### B.3 会话恢复（resume）

`initializeResumedSession`（`acp-agent.ts:1403`）：

```ts
const spawned = await this.spawnProcess();
this.sessionId = handle.sessionId;
if (this.agentCapabilities?.loadSession) {                     // ACP loadSession（会 replay 历史）
  this.replayingHistory = true;
  const response = await this.runACPRequest(() => this.connection!.loadSession({ sessionId, cwd, mcpServers }));
  this.replayingHistory = false;
  this.historyPending = this.persistedHistory.length > 0;
} else if (sessionCapabilities?.resume) {                      // 或 unstable_resumeSession（不 replay）
  const response = await this.runACPRequest(() => this.connection!.unstable_resumeSession({ sessionId, cwd, mcpServers }));
} else { throw new Error(`${provider} does not support ACP session resume`); }
```

重要注释（`:1396`）：某些 ACP provider（如 Devin CLI）要求 `loadSession`/`unstable_resumeSession` 三个参数（sessionId、cwd、mcpServers）**全部**存在，缺一返回 "Invalid params" —— **哪怕 mcpServers 是空数组也要传**。持久化句柄由 `describePersistence`（`:1990`）产出，含 `nativeHandle: this.sessionId`。

### B.4 handshake / 启动超时的设置与处理

`initializeTransport`（`acp-agent.ts:1053`）用 `Promise.race` 同时赛跑三件事 —— initialize 请求、spawn 失败、超时：

```ts
const initializeTimeoutPromise = initializeTimeoutMs
  ? new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error(`ACP initialize timed out after ${initializeTimeoutMs}ms`)), initializeTimeoutMs); })
  : null;
return await this.runACPRequest(() => Promise.race([
  transport.connection.initialize({ protocolVersion, clientCapabilities, clientInfo: { name: "Paseo", version: "dev" } }),
  transport.spawnError,                                  // ← spawn error 直接短路，不用等超时
  ...(initializeTimeoutPromise ? [initializeTimeoutPromise] : []),
]));
```

超时常量：目录探测 `ACP_CATALOG_TIMEOUT_MS = 60_000`，诊断分阶段 `ACP_DIAGNOSTIC_PHASE_TIMEOUT_MS = 20_000`（`:261-262`）。`spawnError` promise 在 `spawnTransport`（`:1025`）里挂 `child.once("error")`，并把 stderr 一起拼进错误消息。spawn 失败时 `spawnProcess`（`:1002`）会 `terminateChildProcess` 清理再抛。

对比 Kivio 缺陷 4：Paseo 把 handshake 超时设成 60s（而非 15-20s），并且用 `Promise.race` 让 **spawn 崩溃能立刻短路**而不是干等超时；超时消息带阶段名（"ACP initialize timed out" vs "ACP session/new timed out"），错误里附 stderr。

---

## C. 流事件归一化与去重（缺陷 2 直接参考）

### C.1 统一事件模型

所有 provider 把输出归一化成 `AgentStreamEvent`（含 `type:"timeline"` + `AgentTimelineItem`：`user_message`/`assistant_message`/`reasoning`/`tool_call` 等）。客户端渲染因此对所有 CLI 一致。

### C.2 ACP：增量 delta 按 messageId 累积，**不做全局前缀去重**

`translateSessionUpdate`（`acp-agent.ts:2445`）分发 session update；`agent_message_chunk` → `createMessageTimelineItem("assistant_message", ...)`（`:2461`）。

`createMessageTimelineItem`（`:2527`）关键：

```ts
const chunkText = contentBlockToText(update.content);
const key = this.messageAssemblyKey(type, update.messageId);
const state = this.messageAssemblies.get(key) ?? { text: "" };
state.text += chunkText;                                  // 内部累积（供 user 回显比对）
this.messageAssemblies.set(key, state);
if (type === "assistant_message") {
  return { type: "assistant_message", text: chunkText,   // ← 只发「本次 delta」，不是累积快照
           messageId: this.resolveAssistantMessageId(update.messageId) };
}
```

**消息边界由 messageId 而非全局前缀识别**：`resolveAssistantMessageId`（`:2560`）—— provider 给了 messageId 就用它；没给就用 `fallbackAssistantMessageId ??= randomUUID()`。而 `fallbackAssistantMessageId = null` 在 **tool_call、agent_thought_chunk、user_message_chunk、current_mode/plan、turn 结束** 时都会被重置（`:2448/2466/2471/2480/2723`）。→ 工具调用后的第二条 assistant 消息会拿到**新的 fallback messageId**，客户端因此把它当成新消息而不是拼到前一条 —— 天然避开 Kivio 缺陷 2 的「跨消息边界失效」。

### C.3 用户消息回显去重：只在当前 turn、只对 user、用前缀匹配

Paseo 对每次提交由 `emitSubmittedUserMessage`（`:2686`）发出**唯一一条**规范 `user_message`（自己的 messageId + turnId），记进 `submittedUserMessageIds` 和 `activeSubmittedUserMessage`。之后 CLI 若回显同一用户消息，`isSubmittedUserMessageEcho`（`:2730`）判定丢弃：

```ts
private isSubmittedUserMessageEcho(item): boolean {
  const active = this.activeSubmittedUserMessage;
  if (!active || active.turnId !== this.activeForegroundTurnId) return false;   // 仅限当前 turn
  if (item.messageId && this.submittedUserMessageIds.has(item.messageId)) return true;
  return active.text.startsWith(item.text);                                     // 仅对 user 做前缀
}
```

`docs/providers.md`（:45）把这条定成硬规则：
> Prefer provider-visible message IDs, but ACP runtimes may omit that ID ... in that case suppress only echo chunks whose accumulated text is a prefix of the active submitted prompt. **Do not perform global transcript text dedupe.**

**这正是 Kivio 缺陷 2 的病根**：Kivio 用一个**全局 `emitted_text` 前缀**去重所有 assistant 输出，跨消息边界必然失效。Paseo 的解法是：前缀去重**只**用于「用户消息回显」，且**限定在当前 turn**（turn 结束即清 `activeSubmittedUserMessage`，`:2724`）；assistant 输出根本不做全局文本去重，靠 messageId/分片 ID 分界。

### C.4 OpenCode：per-part-ID 去重（delta + 累积快照混合的标准解法）

OpenCode 同时发**增量 delta** 和**终态累积快照**，Paseo 用 **分片 ID** 而非文本前缀调和二者（`opencode-agent.ts`）：

- `message.part.delta`（`appendOpenCodeMessagePartDelta`，`:2522`）：流式增量，emit `text: delta` 的同时把分片记入 `state.streamedPartKeys.add(\`text:${partID}\`)` / `reasoning:${partID}`。
- `message.part.updated`（`appendOpenCodeTextPart`，`:2461`；`appendOpenCodeReasoningPart`，`:2498`）：只在 `part.time.end`（分片完成）时处理，且**先查 dedup key**：

```ts
const partKey = resolvePartDedupeKey(part, "text");
if (partKey && state.streamedPartKeys.delete(partKey)) { return; }   // 该分片已流式过 → 丢弃终态快照
if (part.text) events.push({ type: "timeline", item: { type: "assistant_message", text: part.text, messageId: part.messageID }});
```

即：**若某分片已经通过 delta 流式过，就丢弃它的终态累积快照**；只有没走过 delta 的分片（如 resume 时 replay 的历史）才发累积快照。key 是 `${type}:${partID}`，天然跨消息边界成立。用户消息也单独按 `emittedUserMessageIds` 去重（`:2471`）。

→ **这是 Kivio 应对「累积快照 vs 增量 delta」最直接可抄的模式**：给每个流式分片一个 ID，记录已流式集合，终态快照只在其 ID 未出现过时才补发。

### C.5 工具调用事件映射

`tool_call` / `tool_call_update` → `handleToolCallUpdate`（`:2514`）→ `mergeToolSnapshot(toolCallId, update, previous)` 合并快照（按 toolCallId 累积状态）→ `mapToolSnapshotToTimeline`。所有 provider 把工具调用归一到统一的 `ToolCallDetail`（shell/read/edit/write/search…，`docs/architecture.md` "Data flow" 第 6 步）。

---

## D. prompt / 历史组装（缺陷 1 直接参考）

### D.1 只发当前 prompt，历史交给 CLI 原生 session

`startTurn`（`acp-agent.ts:1462`）—— 每轮只把**当前** prompt 发给 CLI：

```ts
this.emitSubmittedUserMessage(prompt, messageId, turnId);        // 发一条规范 user_message
void this.connection.prompt({
  sessionId: this.sessionId,
  messageId,
  prompt: toACPContentBlocks(prompt),                            // ← 只有当前这条，无 transcript
}).then(...).catch(...);
```

历史由 CLI 自己的原生 session 文件保存（B.3 的 resume/loadSession 恢复）。`streamHistory`（`:1524`）只在 resume 后把 `persistedHistory` replay 一次给客户端补时间线，**不参与发给 CLI 的 prompt**。

Claude direct provider 同理：`run`/`startTurn`（`providers/claude/agent.ts:2020/2044`）把 prompt 交给 Anthropic Agent SDK，SDK 用 `resume: sessionId` 续接 `.jsonl` 历史（`:481` `hasResume`），Paseo 不重拼历史。

**对比 Kivio 缺陷 1**：Kivio 在 transcript 里塞了一份最后用户消息，又把 `latest_user_message` 追加了一份，造成重复。Paseo 的架构性答案是：**不要自己组装 transcript**。CLI 原生 session 已经保存全部历史，每轮只需发最新一条 prompt，让 CLI 用 session ID resume。若 Kivio 某些 CLI 确实不支持原生 session、必须重发历史，则应保证 transcript 与「最新消息」二选一，绝不叠加。

### D.2 唯一规范用户消息 = 单一事实源

`docs/providers.md`（:45）：
> Every provider adapter owns its canonical user-message timeline rows. When a foreground prompt is accepted, the adapter must emit exactly one `user_message` timeline item ... Optimistic client messages are UI-only and provider transcript echoes are optional; neither is allowed to be the only source of truth.

即：乐观 UI 消息 + provider 回显都不算数，**适配器自己发的那一条**（`emitSubmittedUserMessage`）才是唯一事实源；provider 回显在当前 turn 内去重掉（见 C.3）。

### D.3 system / instructions 注入

- 通用系统提示由 `composeSystemPromptParts`（`system-prompt.ts`，被 claude/agent.ts:108 引用）组装。
- Pi 用 `--append-system-prompt` 把 Paseo 的 per-agent + daemon 级系统提示**追加**到 Pi 默认编码提示之上（`docs/providers.md`:27），保留 CLI 原生行为。
- 系统注入类 prompt（chat 提及、schedule 触发、notify-on-finish）统一包 `<paseo-system>…</paseo-system>` 信封（`agent-prompt.ts:112` `formatSystemNotificationPrompt`；`:118` `isSystemInjectedEnvelope` 识别），让接收方知道这不是用户 turn。

### D.4 单一发送编排入口

`sendPromptToAgent`（`agent-prompt.ts:177`）是所有发 prompt 路径（WS/MCP/CLI/chat 提及/notify）的唯一编排：`(可选 unarchive) → ensureAgentLoaded → (可选 setMode) → startAgentRun`。注释强调「MUST go through this so behavior can never drift between them」—— 组装逻辑集中一处，杜绝各入口漂移出不同的历史拼接。

---

## E. 错误处理与自愈

### E.1 错误分类与结构化诊断

`summarizeACPRequestError`（`acp-agent.ts:148`）把任意错误归一成 `{ message, code?, diagnostic? }`：ACP JSON-RPC 失败会带 `code` 和 `error.data` 里的 detail，拼成人类可读 message + 结构化 diagnostic 串。turn 失败时（`startTurn` 的 `.catch`，`:1495`）产出：

```ts
const summary = summarizeACPRequestError(error);
this.finishTurn({ type: "turn_failed", provider, error: summary.message, code: summary.code,
                  diagnostic: this.collectDiagnostic(summary.diagnostic ?? summary.message), turnId });
```

`collectDiagnostic`（`:2772`）再补上进程退出码/信号：`exitCode=... | signal=...`。→ 用户看到的不是裸的 "timeout" 字符串，而是带 code + 退出码 + stderr 的分类信息。

### E.2 分阶段健康诊断（provider diagnostic UI）

`buildACPProbeDiagnosticRows`（`acp-agent.ts:1127`）把探测拆成**四个可观测阶段**，每阶段独立超时（`ACP_DIAGNOSTIC_PHASE_TIMEOUT_MS=20_000`）、各自 try/catch、失败即返回已积累的行并附 stderr：

```
ACP spawn        → ok (123ms) | error: ...
ACP initialize   → ok (45ms)  | error: ...  (+ ACP stderr 行)
ACP session/new  → ok (200ms; models=5; modes=3) | error: ...
ACP cleanup      → ok (10ms)
```

direct provider 也有对应 `getDiagnostic()`（`agent-sdk-types.ts` 可选方法；Claude 实现见 `providers/claude/agent.ts:1544`）：拼 PATH 匹配、二进制版本、Auth 状态行（`diagnostic-utils.ts` 的 `buildCommandResolutionDiagnosticRows`/`buildBinaryDiagnosticRows`/`formatProviderDiagnostic`）。→ Kivio 缺陷 4 想要的「分类 + 恢复引导」，Paseo 用一套可复用的诊断行基础设施（`diagnostic-utils.ts`）承载。

### E.3 取消/崩溃/自愈

- **取消需 provider 确认**（`docs/agent-lifecycle.md`:35）：只有在 provider ack interrupt 或发出终态 turn 事件后才改生命周期状态。若 interrupt 被拒或超时，agent 保持 `running`，后续 replace/reload/rewind/Stop 必须报告失败而非假装成功 —— 明确警告「不要在没有 provider ack 时本地合成取消，会造成 split-brain」。
- **进程注册表对账**（`docs/providers.md`:51-57）：能跨 session 存活的 helper 进程要登记到 daemon 的 managed-process registry（PID + 启动命令 + 进程身份）。daemon 启动时后台对账：死 PID 删除、PID 身份不符删除（不杀）、只终结确认属于 Paseo 的残留、无法检查的记录留待下次。明确「不要加宽泛的按进程名清扫」。
- **错误态可恢复**：snapshot 的 `error` 态缓存到显式刷新为止（A.4），用户点「刷新」才重探，避免坏态自动无限重试。
- **stderr 捕获贯穿**：`spawnTransport`（`:1020`）挂 `child.stderr.on("data")` 累积 `stderrChunks`，spawn error/诊断都把它拼进去。

---

## F. 值得抄的其他设计

### F.1 传输层逐行 JSON 容错（缺陷 3 的根本解法）

**ACP stdio 传输**（`acp-agent.ts` 内 `createLoggedNdJsonStream`，NDJSON 解析在 `:300` 起）：

```ts
content += textDecoder.decode(value, { stream: true });
const lines = content.split("\n");
content = lines.pop() || "";                              // 半行留到下次
for (const line of lines) {
  const trimmedLine = line.trim();
  if (!trimmedLine) continue;
  try {
    const message = JSON.parse(trimmedLine);
    controller.enqueue(normalizeACPIncomingMessage(message));
  } catch (error) {
    options.logger.warn({ err: ..., provider }, "ACP agent emitted non-JSON stdout; ignoring line");  // ← 只跳这一行
  }
}
```

**Codex app-server 传输**（`providers/codex/app-server-transport.ts:289` `handleLine`）同款模式：`readline` 逐行 + `JSON.parse` try/catch → `"Ignoring non-JSON Codex app-server stdout line"`，坏行只 warn 不中断。

→ **这是 Kivio 缺陷 3 的直接修法**：把「读到一行非 JSON banner 就整体放弃探测」改成「逐行解析，坏行 warn 跳过，JSON-RPC 响应（含 initialize/newSession/configOptions）照常从后续行到达」。ACP 探测因此对启动 banner 免疫。

### F.2 `withTimeout` 工具无处不在

`packages/server/src/utils/promise-timeout.ts` 的 `withTimeout(promise, ms, message)` 包裹每个可能挂起的探测/请求（catalog、availability、diagnostic、session/new），超时消息带阶段名和 provider label。Kivio 可引入同款薄封装统一超时语义。

### F.3 单飞锁（single-flight）防重复探测

ProviderSnapshotManager 用 `providerLoads` map 做 per-(cwd,provider) 单飞（A.4）。对应 Kivio CLAUDE.md 里已提到的 `availability_probe_lock` / `model_probe_lock_for`（Kivio 已部分借鉴此模式，见 `.trellis/tasks/07-19-external-cli-detection-cache`）。

### F.4 provider-manifest / registry / snapshot 三层分离

- `provider-manifest.ts` —— UI 元数据（icon、colorTier、默认 mode），静态脚手架。
- `provider-registry.ts` —— `PROVIDER_CLIENT_FACTORIES` 工厂表，`(logger, runtimeSettings, options)` 造 client。
- `provider-snapshot-manager.ts` —— 运行时探测结果缓存。
文档 Gotcha 明确：manifest 的静态 modes 只用于 UI 脚手架，**运行时从 agent 进程拿到的才是模型/模式的事实源**。

### F.5 协议验证（zod-aot，`docs/protocol-validation.md`）

客户端用 zod-aot 生成的验证器替代热路径上的运行时 Zod（Hermes 上 353KB 快照从 10.9ms/5.9MB 降到 2.5ms/1.2MB）。与 Kivio 四缺陷关系不大，但示范了「schema 保持纯结构声明、规范化放到显式消费者」的边界纪律。

---

## G. 对 Kivio 改造的具体建议映射

下表把 Paseo 的可借鉴机制对应到 Kivio 四缺陷与 `src-tauri/src/external_agents/` 具体文件（Kivio 侧文件路径基于 CLAUDE.md 描述与实际目录 `run.rs / prompt.rs / session/acp.rs / detection.rs / commands.rs / stream/*` 等）。

### 缺陷 1（prompt 重复拼两次）→ 参考 D 节

- **Paseo 原则**：`startTurn` 只发当前 prompt（`acp-agent.ts:1485`），历史由 CLI 原生 session resume 恢复；唯一规范 user_message 由适配器自发（`:2686`）。
- **Kivio 落点**：`external_agents/prompt.rs`（prompt 组装）+ `external_agents/context.rs` / `compact.rs`（上下文组装）。改造方向：让 transcript 与 `latest_user_message` **二选一**——若走 CLI 原生 session（ACP loadSession / Claude `--resume` / Codex rollout），则只发最新一条，删掉 transcript 里的重复末条；若某 CLI 无原生 session 必须重发历史，则最新消息不再单独追加。
- **集中入口**：仿 `sendPromptToAgent`（`agent-prompt.ts:177`）把所有发送路径收敛到 `run.rs` 单一函数，杜绝多入口拼接漂移。

### 缺陷 2（ACP 累积快照全局前缀去重跨消息失效）→ 参考 C 节

- **Paseo 原则**：**不做全局文本去重**；assistant 输出按 messageId/分片 ID 分界（`acp-agent.ts:2560`，边界事件重置 fallbackId）；OpenCode 用 **per-part-ID `streamedPartKeys`** 调和 delta 与终态快照（`opencode-agent.ts:2461/2485/2522`）；用户回显前缀去重仅限当前 turn（`:2730`）。
- **Kivio 落点**：`external_agents/session/acp.rs`（ACP `agent_message_chunk` 累积/去重）+ `external_agents/stream/mod.rs`。改造方向：
  1. 删掉单一全局 `emitted_text` 前缀去重。
  2. 按 ACP `messageId`（缺失时用 turn 内递增的 fallback id，并在 tool_call/thought/turn 边界重置）给每条 assistant 消息分界，客户端按 id 累积/替换。
  3. 若上游发累积快照，改用「已流式分片 ID 集合」判定：分片 ID 已流过则丢弃其终态快照（抄 `resolvePartDedupeKey` + `streamedPartKeys` 模式）。
  4. 用户消息回显去重限定在当前 turn（turn 结束清状态）。

### 缺陷 3（模型探测遇非 JSON banner 整体放弃、静默降级）→ 参考 A.3/A.5 + F.1

- **Paseo 原则**：传输层逐行 `JSON.parse` + try/catch，坏行只 warn 跳过（`acp-agent.ts:300`、`codex app-server-transport.ts:289`）；模型来源双通道（session `availableModels` + `configOptions[category=model]`，`:644`）；探测失败进 `status:"error"` 携带消息，不塞静态 fallback（`provider-snapshot-manager.ts:810`）。
- **Kivio 落点**：`external_agents/session/acp.rs`（探测 initialize→session/new→读 configOptions/availableModels）+ `external_agents/detection.rs`（探测编排）+ `external_agents/commands.rs`（`chat_detect_external_agent_models`）。改造方向：
  1. NDJSON 解析改为「按行解析，非 JSON 行 warn 后 `continue`」，不因单行 banner 放弃整个模型探测。
  2. 同时读 ACP 的 `availableModels` 与 `configOptions` 里 `category:"model"` 的 select，任一有值即用。
  3. 探测失败/超时时向前端返回**显式 error 态 + 错误消息**，让 UI 区分「探测失败」「未登录」「无模型」，而不是永远只显示 "Default"。

### 缺陷 4（ACP handshake 15-20s 超时、鉴权失败裸错误直显）→ 参考 B.4 + E 节

- **Paseo 原则**：initialize 用 `Promise.race(initialize, spawnError, timeout)` 让 spawn 崩溃立即短路（`acp-agent.ts:1053`）；handshake/catalog 超时 60s、诊断阶段 20s（`:261`，可 env 调）；错误经 `summarizeACPRequestError` 分类成 `{message, code, diagnostic}`（`:148`）+ `collectDiagnostic` 补退出码/信号（`:2772`）；四阶段诊断 UI（`:1127`）+ 可复用诊断行基础设施（`diagnostic-utils.ts`）；取消需 provider ack（`agent-lifecycle.md:35`）。
- **Kivio 落点**：`external_agents/session/acp.rs`（handshake）+ `external_agents/spawn.rs`（进程 spawn）+ `external_agents/run.rs`（错误呈现）+ `external_agents/session/live.rs`。改造方向：
  1. handshake 超时上调（Paseo 用 60s），并用 select/race 让 spawn error 与 stderr 立即短路，不干等超时。
  2. 错误分类：鉴权失败 / 未安装 / 超时 / 进程崩溃分门别类，带上退出码 + stderr 摘要，给用户可操作的恢复引导（如「运行 `xxx login`」）。
  3. 引入分阶段探测诊断（spawn/initialize/session-new），把裸 "ACP handshake timeout" 换成带阶段 + stderr 的诊断串。
  4. 探测/session 进程注册与清理对账，避免残留（参考 managed-process registry）。

---

## Caveats / Not Found

- Paseo 内建 provider **不含 grok**；`gemini` 在 Kivio 侧存在（`defs/grok.rs`/无 gemini def），但 Paseo 的 gemini 支持未在本次阅读的内建 provider 列表中出现（Paseo 内建为 claude/codex/copilot/cursor/opencode/pi/omp + generic-acp/kiro/trae）。cursor 在 Paseo 是 ACP wrapper（`cursor-acp-agent.ts`，仅 45 行，全靠基类）。跨 CLI 的通用做法仍完全适用。
- Paseo 是 daemon+WebSocket 的 client-server 架构，Kivio 是 Tauri 单进程；「运行时驻留回收」「跨 client tab」等 daemon 特性不必照搬，但 prompt 组装、流去重、探测容错、错误分类四点是纯适配器层逻辑，可直接迁移。
- 本报告未逐行通读 `opencode-agent.ts`(4287 行) / `codex-app-server-agent.ts`(6593 行) 全文，聚焦于与四缺陷相关的流去重、handshake、探测、错误路径；如需 Codex AppServer 的完整 JSON-RPC 握手细节可深入 `providers/codex/app-server-transport.ts`。
- 前端（`packages/app` timeline reducers `session-stream-reducers.ts`）如何按 sequence/messageId 做客户端级 dedup 未展开，Kivio 归一化主要发生在 Rust 后端，故重点放在 server 侧适配器。
