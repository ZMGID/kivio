# 本地 CLI 检测缓存重构

## Goal

消除「换会话就一直在检测本机 CLI」的卡顿。参照 Paseo（`getpaseo/paseo` 的 `ProviderSnapshotManager`）：把**可用性检测**（装没装/版本/登录，与 cwd 无关）和**模型目录探测**（昂贵，claude 超时达 25s）彻底拆开；可用性全局缓存、换会话直接命中；模型只对**选中的那个 CLI 懒查**；并发去重；先返旧值不闪"检测中"。

## Background

现状三坑（调研见 `research/`）：
1. `RuntimePicker`/`PermissionPicker`/`ExternalModelSelector` 的 `useEffect([conversationId])` 每切会话都调 `detectExternalAgents`，且先 `setAgents([])` 闪一下。
2. 检测缓存 `external_detected_agents_cache` 按 **cwd** 存，而每会话 cwd 不同（`chat-workspaces/<id>`）→ 换会话必失效。
3. `detect_all_agents` 对**所有** 6 个 CLI 都跑 version + auth(5s) + 模型探测（claude ClaudeInit 25s、ACP 起探测会话）。

Paseo 对照：可用性 PATH 全局 + 廉价 `--version`；模型 `fetchCatalog` 懒查、只查选中的 provider、single-flight、事件驱动失效（无 TTL）；先返 cached/loading 后台 warm。

## Requirements

- R1: 拆两层。**可用性**（binary + version + auth）与 cwd 无关，独立探测。**模型目录**按 (agent, cwd) 单独探测。
- R2: 可用性缓存用**全局 key**（非 cwd），TTL 拉长（≥10min）。换会话命中缓存，不重测。`force_refresh` 仍可手动重测（设置页用）。
- R3: `chat_detect_external_agents` **不再对所有 CLI 跑模型探测**；`models` 字段回填「已缓存的模型」或 `fallback_models`，不触发新探测。
- R4: 新增按需模型探测入口（命令）：只探**一个**指定 agent 的模型（cwd-scoped），缓存 (agent, cwd)。前端在**选中该 agent / 打开其模型下拉**时才调。
- R5: **single-flight**：并发的可用性检测只实跑一次（RuntimePicker+PermissionPicker+ModelSelector 同一次会话切换会并发三调）；同 (agent,cwd) 的模型探测也去重。
- R6: 前端不再 `setAgents([])` 清空重置 → 保留上次结果、后台刷新，不闪"检测中"。
- R7: 行为兼容：设置页 `ExternalAgentsSettings` 的完整列表 + 手动刷新仍能拿到每个 agent 的模型（可用 force 或对每个 available agent 触发模型探测）。

## Non-Goals

- 不引入完整的 ProviderSnapshotManager / 事件推送架构（kivio 用长 TTL + 手动刷新即可，YAGNI）。
- 不改附件功能（另一任务）。
- 不改各 CLI 的探测协议本身（detect_acp_models / detect_claude_models 等不动）。

## Acceptance Criteria

- [ ] AC1: 连续切换 5 个不同会话，第 2 次起可用性**不再重跑子进程**（缓存命中）；无"检测中"闪烁。
- [ ] AC2: 冷启动后首次进聊天，只跑一次可用性探测（不对 6 个 CLI 跑模型探测）；列表快速出现。
- [ ] AC3: 选中某个外部 CLI 后打开模型下拉，才触发该 agent 的模型探测；其余 agent 不探测。
- [ ] AC4: 并发触发检测（多组件同时挂载）只实跑一次可用性探测（single-flight 生效）。
- [ ] AC5: 设置页手动刷新仍列出所有 available agent 的模型。
- [ ] AC6: 纯可用性探测不含 25s 的 claude 模型探测；单次可用性探测墙钟显著下降。
- [ ] AC7: `cargo test` 相关单测通过（缓存 key 全局化、拆分后回填逻辑、single-flight）。
