# 修复性能与资源生命周期问题

## Goal

系统性修复 Kivio 中已确认的性能热点和资源生命周期缺口，使窗口、子进程、线程、Worker、网络监听器、后台任务及长期缓存都能在超时、停止、退出或空闲时及时释放；同时降低长会话流式渲染的每帧开销，避免应用长时间运行后残留内存持续增长。

本任务直接实施全部已确认问题，不再拆分范围或等待额外决策。

## Background

审查基线已通过：

- `npm run lint`
- `npm run typecheck`
- `npm test -- --run`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`（1480 passed，0 failed，12 ignored）

已确认 Chat、Lens、Translate、设置/翻译窗口、macOS overlay、MCP/外部 CLI 活会话及大多数前端 Tauri listener 已有正确清理路径；本任务不重写这些正常实现，只补齐以下缺口。

## Requirements

### R1. PATH 探测超时必须回收子进程与等待线程

现状：`src-tauri/src/path_env.rs:77` 超时返回后没有终止 shell 子进程，也没有等待负责 `wait_with_output` 的线程结束。

要求：

- 超时后终止对应子进程，并确保等待线程可结束、可 join。
- 正常完成、spawn 失败、kill 失败和超时路径都不得留下 zombie 或永久后台线程。
- 保持现有 PATH 探测结果和超时降级行为兼容。

### R2. Pyodide 执行超时必须统一并释放 Worker

现状：Rust 侧 `src-tauri/src/mcp/registry.rs:985` 等待 `timeout + 5s`，前端 `src/chat/pyodideClient.ts:88` 等待 `timeout + 120s`；后端先超时后，Pyodide 仍可能继续执行并占用数百 MB。Worker 被终止时，其他 pending 请求也没有立即统一拒绝。

要求：

- Rust、前端 Worker 和脚本执行使用一致、可解释的超时预算，不允许前端比后端多运行两分钟。
- 任一请求导致 Worker 被终止或重置时，立即拒绝并清空所有 pending 请求及 timer。
- Worker error、message error、显式 dispose、超时重启均走同一清理函数。
- 后续请求能够按需创建新 Worker，不因一次超时永久失效。

### R3. macOS OCR helper 必须支持空闲退出和应用退出清理

现状：`src-tauri/src/macos_ocr.rs:164` 启动的 helper 以及 `src-tauri/swift/kivio-ocr-helper/Sources/main.swift:65` 的服务循环在首次使用后长期常驻。

要求：

- helper 在一段可配置或常量化的空闲期后主动退出。
- Rust 侧在应用退出或管理器销毁时关闭 stdin、终止并 reap 子进程。
- 请求期间不得被空闲计时器误杀；helper 退出后下一次 OCR 可透明重启。
- 异常退出、管道损坏和 shutdown 路径不得残留 child handle。

### R4. Preview 停止必须关闭完整服务生命周期

现状：`src-tauri/src/plugins/preview.rs:160` 的 `stop_all_previews()` 只清理预览进程，没有停止 TCP listener、SSE 连接/心跳任务，也未清空 `RELOAD_TX` 和 `server_port`。

要求：

- Preview server 持有显式 cancellation/shutdown handle。
- `stop_all_previews()` 同时停止 listener、SSE/heartbeat 后台任务和预览进程。
- 停止后清空 reload sender、端口和其他全局状态，旧 SSE 客户端及时断开。
- 多次 start/stop 幂等，停止后可在新端口或可用端口重新启动。

### R5. 外部 CLI cwd 缓存必须有界并自动过期

现状：`src-tauri/src/state.rs:219` 的三个按 cwd 缓存没有容量上限，也没有全局过期清扫；访问大量目录后会永久保留历史 key/value。

要求：

- 三个缓存统一使用容量上限与 TTL 清扫策略。
- 读写时执行低成本惰性过期清理；容量超限时按确定性策略淘汰最旧或最久未访问项。
- 不改变现有缓存命中语义和 CLI 检测结果。
- 有测试覆盖 TTL、容量、更新和并发访问。

### R6. 知识库锁表不得永久保留历史 kb_id

现状：`src-tauri/src/chat/knowledge_base/ingest.rs:204` 的静态锁表为每个历史 `kb_id` 永久保存锁对象。

要求：

- 锁表使用可回收的弱引用或等价机制；无任务持有某知识库锁时允许条目被清理。
- 同一 `kb_id` 的并发入库仍必须串行，不得因回收策略产生双锁竞态。
- 在删除库、完成任务或后续访问时清扫失效项。
- 有并发与回收测试。

### R7. 长会话流式消息渲染不得每帧重建全部派生数据

现状：`src/chat/MessageList.tsx:222` 每个流式帧都会重建完整 history items、navigator 索引并同步贴底滚动；长会话形成 O(N)/frame 开销，测试中还反复出现 virtua 的 `flushSync was called from inside a lifecycle method` 警告。

要求：

- 将只依赖稳定消息结构的昂贵派生数据与流式文本更新解耦，避免每个 token/frame 全量重建导航索引和非活跃历史项。
- 贴底滚动合并到每动画帧至多一次，并避免在 React lifecycle 中触发 virtua 同步刷新警告。
- 保持虚拟列表、分支导航、历史分页、工具调用卡片和自动贴底行为一致。
- 添加长列表流式更新测试或可重复的渲染计数断言，确保优化不是仅凭人工观察。

### R8. MCP 启动预热必须限制并发与常驻资源

现状：`src-tauri/src/lib.rs:382` 启动时并行预热所有启用的 MCP server，默认会话可常驻约 10 分钟；配置较多时会造成启动阶段 CPU/内存/子进程尖峰。

要求：

- 预热采用小并发上限，或只预热明确需要的 server；不得无界并发启动全部 server。
- 单个 server 失败或超时不阻塞应用启动，也不阻止其他 server 按需连接。
- 保留首次工具调用可按需启动的兜底路径。
- 有测试或可验证日志证明并发上限生效。

## Constraints

- 保留用户当前未提交修改：`src/index.css`、`src/settings/SettingsShell.tsx`、`src/settings/components.tsx`、`website/DEPLOY.md`、`website/deploy.sh`。
- 不改变正常窗口关闭语义，不引入“隐藏窗口代替销毁”的新常驻行为。
- 清理操作必须幂等；应用退出路径不得因重复 stop/kill/purge panic。
- 超时和后台清理不得阻塞 Tauri 主线程或 React 渲染线程。
- 不为本任务引入大型运行时依赖；优先使用现有 Tokio、标准库和前端调度能力。
- 日志不得输出用户脚本正文、知识库内容、环境变量或凭据。

## Acceptance Criteria

- [x] AC1（R1）：PATH 探测超时测试证明 child 被终止并 reap，等待线程结束；正常探测行为不变。
- [x] AC2（R2）：Pyodide 超时时 Rust/前端预算一致；Worker 被重置后所有 pending promise 立即拒绝、timer 清空，下一次调用可重建成功。
- [x] AC3（R3）：OCR helper 空闲后退出，应用退出时强制清理；退出后的下一次 OCR 能透明重启。
- [x] AC4（R4）：停止 Preview 后 listener 端口释放、SSE/heartbeat 结束、全局 sender/port 清空；重复 start/stop 通过测试。
- [x] AC5（R5）：三个 cwd 缓存均通过容量、TTL、更新与并发测试，历史目录不会无限增长。
- [x] AC6（R6）：知识库锁表只保留活锁；同库并发仍串行，不同库可并行，历史 `kb_id` 可回收。
- [x] AC7（R7）：长会话流式更新不再每帧重建全部导航/历史派生数据；自动贴底正确，相关测试不再产生 lifecycle 内 `flushSync` 警告。
- [x] AC8（R8）：MCP 预热并发有明确上限，失败不阻塞启动，未预热 server 仍可按需启动。
- [x] AC9：所有资源清理 API 可重复调用且无 panic；应用退出集成路径覆盖 Preview、OCR、Pyodide 和外部会话。
- [x] AC10：`npm run lint`、`npm run typecheck`、`npm test -- --run`、`cargo test --manifest-path src-tauri/Cargo.toml --lib` 全部通过。

## Out of Scope

- 重写已确认正常的 Chat/Lens/Translate/设置窗口生命周期。
- 更换前端虚拟列表库或重构整个聊天消息数据模型。
- 更换 Pyodide、OCR 或 MCP 技术栈。
- 对未观察到泄漏证据的全局单例做无差别清除。
