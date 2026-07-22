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

| 子任务 | 覆盖缺陷（kivio-audit 编号） | 状态 |
|---|---|---|
| `07-20-cli-message-chain` 消息链路正确性 | 1（prompt 重复）、2（ACP 去重）、C1、N7、N9 + 对应测试缺口 | **完成**（ce76f60，真机验收通过 07-22） |
| `07-20-cli-session-lifecycle` 会话生命周期与自愈 | N1（stderr 挂死）、4（错误裸奔）、N3（换模型无效）、A3/A5/A6/A4、C2/C3、B3 | **完成**（4214956，真机验收通过 07-22） |
| `07-20-cli-detection-models` 检测/模型探测/前端 | 3（探测放弃）、N2（每轮探测）、N4、B2/B4、D1/D2/D3、F2/F4、N5 | **完成**（3487e05 + 验收反馈修复 268d328/d86e355/6289822/254bd72/b52c8ce） |
| `07-20-cli-native-sessions` pi/kimi 原生会话 + 会话-CLI 绑定 | 用户补充需求：废除历史重放、pi --session-id、kimi 迁 ACP、CLI 绑定不可切换 | **完成**（3456997 + 真机修复 2594657/821e221，验收通过 07-22） |

依赖关系同时写入各子任务 prd.md；父任务本身无直接实现工作，负责收口验收。

## Deferred（本轮明确不做、复检发现的病态边界）

- 持久会话 actor 卡死在 stdin 写（管道满 + 子进程 SIGSTOP）时 Close 命令无人消费，兜底退化为 App 退出回收（session-lifecycle 复检观察 3）。
- mid-turn 进程死亡的错误串不含 stderr 尾部（tail 只在 connect/close 路径 join；随后的重连失败会带上）。
- ACP 模型探测固定多等 1.5s 通知窗口（懒查 + 缓存下可接受；若要消除需探测协议里的"目录已完整"信号）。
- Paseo 式 managed-process registry 对账（进程残留清扫）未引入。
- kimi `--add-dir` 未接（kimi 已迁 ACP，cwd 即工作区，不再需要）。
- pi/kimi 存量会话升级后一次性上下文丢失（旧版靠重放、无原生 session 可续），静默失忆 → 发布说明提示。
- 持久协议 reuse 轮不带附件路径说明块；claude/pi resume 轮 skip_instructions 时 skill body 一并被 skip（均为存量行为，复检 1a 观察项）。

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

## 收口记录（2026-07-22 真机验收）

全部子任务归档。真机验收结果：codex/grok 模型同步显示 ✓；消息不重复 ✓；第 2+ 轮前置 3-7ms ✓；
pi 42 / kimi 7 两轮记忆 + 跨 App 重启续接 ✓（第 2 轮上行仅数百 tokens，证明只发最新消息）；
会话-CLI 绑定置灰 ✓。验收中另修 2 缺陷：pi agent_end 后转圈不止、EPIPE 假性"生成异常结束"。
E1（登出错误分类）/E2（中途换模型）未逐项真机验证，有单测覆盖，出问题按 errors.rs/acp.rs 契约排查。
