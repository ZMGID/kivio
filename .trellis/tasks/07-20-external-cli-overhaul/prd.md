# PRD — 外部 CLI Agent 连接与消息链路改造（父任务）

## 背景

外部 CLI Agent 子系统（`src-tauri/src/external_agents/`）当前处于"不可用"体验状态，用户报告三类现象：

1. 模型/推理下拉永远显示两个 "Default"；
2. 会话报错原文直显："Grok CLI 读取输出失败：Authentication required / ACP handshake timeout"；
3. 用户消息被发两遍（CLI 回复"你发了两遍 123"）、助手回复显示两遍。

两份研究报告是本任务的需求来源：
- `research/kivio-audit.md` — 全面缺陷审计：26 项 = 阻断 1 / 严重 8 / 一般 17（含缺陷总表）。
- `research/paseo-reference.md` — 参考项目 Paseo 的成熟做法，G 节逐条映射到 Kivio 文件。

## 目标（用户可感知）

- 发消息不再重复、回复不再重复显示（任意 CLI、任意轮次、含工具调用后）。
- 选中已登录的 CLI 后能看到真实模型列表；探测失败时 UI 明确显示"降级/失败 + 重试"，而不是假装只有 Default。
- 会话错误分类呈现（未登录 → 引导登录命令；超时 → 自动重连一次再报）；不再出现裸协议错误串。
- 每轮发消息不再有几秒~十几秒的无谓探测延迟（消除 run.rs:88 每轮完整模型探测）。
- 会话中途切模型/推理档对所有协议真实生效（或明确重连）。
- 持久会话不再有 stderr 管道写满挂死的风险。

## 非目标

- 不新增 CLI（现有 defs 不变）；不引入 Paseo 的 daemon/WebSocket 架构；不做 MCP 透传到外部 CLI；不改内置 agent loop（`chat::agent::run_agent_loop` 与 `AgentHost` trait 零改动）。

## 子任务地图（每个独立可验收）

| 子任务 | 覆盖缺陷（kivio-audit 编号） | 依赖 |
|---|---|---|
| `07-20-cli-message-chain` 消息链路正确性 | 1（prompt 重复）、2（ACP 去重）、C1、N7、N9 + 对应测试缺口 | 无（先行） |
| `07-20-cli-session-lifecycle` 会话生命周期与自愈 | N1（stderr 挂死）、4（错误裸奔）、N3（换模型无效）、A3/A5/A6/A4、C2/C3、B3 | 在 message-chain 之后（同文件 acp.rs/run.rs，串行避免冲突） |
| `07-20-cli-detection-models` 检测/模型探测/前端 | 3（探测放弃）、N2（每轮探测）、N4、B2/B4、D1/D2/D3、F2/F4、N5 | 与前两者文件交集小，可在 message-chain 后并行于 session-lifecycle |

依赖关系同时写入各子任务 prd.md；父任务本身无直接实现工作，负责收口验收。

## 跨子任务验收标准（父任务收口时全部满足）

1. **消息正确性**：对 grok（ACP 持久）、claude（stream-json + resume）、pi（一次性全量回放）三个代表协议各跑一次含工具调用的 3 轮对话：用户消息在 CLI 侧收到恰好一次；助手正文在 UI 显示恰好一次。
2. **模型列表**：grok 登录态下模型下拉出现非 Default 的真实模型；人为制造探测失败时 UI 出现降级标识与重试入口，不静默。
3. **性能**：外部 CLI 会话第 2+ 轮，从点发送到 CLI 收到 prompt 的后端耗时 < 500ms（不含 CLI 自身响应）。
4. **错误呈现**：mock/真实鉴权失败后发消息，气泡显示分类后的可操作中文提示，不含裸 "ACP handshake timeout" / 原始 RPC 错误串。
5. **回归**：`cargo test --manifest-path src-tauri/Cargo.toml` + `npm run lint` + `npm run typecheck` + `npm test` 全绿；audit E 节测试缺口中缺陷 1/2/3 至少各补一条单测。
6. **抽象完整**：一次性 `run_acp_session` 与持久 `AcpSession` 的重复逻辑合并为共享核心（同一 bug 不存在两份拷贝）。

## 约束

- Rust 错误沿用 `Result<_, String>`；子进程一律 `no_console_window()`；`chat-stream`/`chat-tool` 事件 payload 形状是 UI 契约不可改。
- 每个子任务的验收由主会话执行，不由实现子代理自证。
