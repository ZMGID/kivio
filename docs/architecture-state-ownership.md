# AppState 状态所有权迁移清单

本清单对应 [`architecture-convergence-spec.md`](./prd/architecture-convergence-spec.md) 的 R3（延续前期 B3）。`AppState` 是应用组合根；持有领域句柄不代表允许调用方拿到领域 map、锁或任意写入字段。表中的“已迁移”指代码所有权与调用方已接线；测试结果和平台验收以本批交付记录为准。

## R3-C：缓存、后台作业与平台生命周期

| 原始可变字段 | 新 owner / Interface | 调用方迁移 | 生命周期与失效 | 锁顺序 / 等待约束 | 状态 |
|---|---|---|---|---|---|
| `external_slash_commands_cache` | `external_agents::discovery_state::ExternalDiscoveryState`；get/set cache | `slash` 等沿用 AppState 行为转发；没有 cache/map getter | 进程内；空结果短 TTL、非空长 TTL；容量 64，保留 LRU 清理 | 单次 map 锁，不跨探测 await | 已迁移 |
| `external_agent_models_cache` | 同上；get/set/agent-scoped invalidate/all invalidate | `external_agents/commands`、设置保存与 profile 清理通过行为方法 | 保留 probed / fallback 分别 TTL；按 `agent:cwd` 隔离；更换设置失效 | map 锁只做内存操作；探测 permit 先于缓存复查 | 已迁移 |
| `external_detected_agents_cache` | 同上；get/set/clear availability cache | 可用性命令和后台刷新 | 进程内 TTL；原有磁盘 availability snapshot 路径与刷新策略保留 | map 锁不包围磁盘快照 I/O 或子进程 | 已迁移 |
| `availability_probe_lock` | 同上；`acquire_availability_probe` / `try_acquire_availability_probe` | `external_agents/commands.rs` 两处等待/跳过路径 | 一次可用性探测期间持有异步 permit；退出或取消即释放 | 允许异步探测持有专用 permit；无同步 map guard 跨 await | 已迁移 |
| `model_probe_locks` | 同上；`acquire_model_probe(key)` | 模型探测直接获取 owned permit；不再暴露 `Arc<Mutex>` | 相同 key single-flight，不同 key 并行；key 表仍按原实现存活到进程退出 | 短持 map 锁找到 permit，先释放 map 再 await permit | 已迁移 |
| `key_cooldowns`、`active_key_idx` | `chat::provider_runtime::ProviderRuntimeState`；pick/fail/ok/prefer/sync | provider model 与设置保存保留原行为调用，AppState 只转发 | 初始化读取规范设置；60 秒冷却；provider 删除清理；无关设置变化保留 failover 指针 | pick 按 cooldown → active；其他写操作分段释放，不存在反向嵌套；无 I/O/await | 已迁移 |
| `prompt_cache_key_unsupported`、`prompt_cache_retention_unsupported`、`reasoning_replay_unsupported` | 同上；各 capability 的 query/mark | OpenAI / Responses adapters | 按 endpoint 记录本进程观察到的拒绝；三类能力独立；重启清空 | 各集合私有短锁；不持锁调用 provider | 已迁移 |
| `image_route_cache` | 同上；`image_route` / `remember_image_route` | `chat/image_generation.rs` 两处裸读写已删除 | `(provider_id, normalized_model)` 隔离；仅备用端点成功后记忆；进程内 | 锁仅围绕 get/insert；请求前释放 | 已迁移 |
| `background_commands` | `native_tools::background_registry::BackgroundCommandRegistry`；register/snapshot(s)/complete/kill/clear/kill-all/kill-conversation | `native_tools/shell.rs` 全部调用（含 detached waiter）；`chat/commands/interaction.rs` 列表与清理；AppState 注册与退出转发 | 跨 turn 存活；显式 kill、删所属对话、退出 sweep 清理；waiter 持 `Arc<owner>`；运行中条目不因容量被淘汰 | 查找、终态转换、take kill sender、移除归属在一个短锁内；释放后发送/杀进程/删日志；Killed 不被迟到 completion 覆盖 | 已迁移 |
| `external_background_tasks` | `external_agents::background_tasks::ExternalBackgroundTasks`；upsert/reconciled snapshot/clear finished | `external_agents/run` 通过既有 upsert；后台任务面板两处裸访问已删除 | 仅观察 CLI 自有任务；终态补齐不丢 kind/description；无活会话的 running 转 stopped；清理按对话隔离 | 与既有路径一致：外部任务表 → 活会话可用性短读；不 await、不发控制命令；反方向不得嵌套 | 已迁移 |
| `request_debug` | `chat::request_debug::RequestDebugState`；record/snapshot/clear | provider 调试采集、面板命令经既有函数进入 owner | 内存上限 50；保留已有 `request_debug/records.jsonl` 镜像；clear 清内存和镜像；退出清内存，磁盘镜像按原有行为保留 | mirror gate → buffer；释放 buffer 后镜像 I/O；record/clear 同一 gate 保证磁盘顺序不倒退；无 await | 已迁移 |
| `prev_frontmost_pid_lens`、`prev_frontmost_pid_translator` | `window_focus::FrontmostAppState` 的两个私有 `FocusReturnSlot`；remember/previous/take/forget | `windows` 平台操作、`commands`、`lens_commands`、`lib`、`shortcuts` 全部 16 个原字段调用 | Lens 与输入翻译各自保存前台身份；self/invalid PID 不保存；restore 原子取出清零；reassert 只读；显式打开 Chat 清槽 | 保留 store/load/swap 的 `SeqCst`；主线程调用与 activate 条件不变；调用方不能获取 AtomicI32 | 已迁移；macOS 实机另验 |
| `settings_save_lock` | `settings::SettingsPersistenceGate`；`begin_full_save`，AppState `begin_settings_save` | `commands.rs` 全对象保存入口 | 完整保存及工作区迁移期间持异步 permit；退出/取消释放；轻量写仍不经过此 gate | full-save permit → 短 settings 读/CAS；轻量写以 revision 使旧完整保存冲突，不扩大锁范围 | 已迁移 |

## 其他领域与本批组合检查

| 状态组 | 当前 owner | 调用与生命周期约束 | 批次 |
|---|---|---|---|
| Chat generation、active replies、steering/follow-up/goal-user-queue、创建会话与 popout 协调 | `chat::runtime_state::ChatRuntimeState` | 每 run 自然结束与全对话取消不同；send 检查与占位保持同一原子区间；无裸索引外泄 | R3-A |
| Chat protocol replay、subscribers | `chat::protocol::ChatProtocolState` | 保留 replay/cursor/事件归属；协议转换与窗口 channel 生命周期由 owner 管理 | R3-A |
| live external sessions | `external_agents::session::live::LiveSessionRegistry` | 控制通道与会话复用/移除归 owner；不跨 await 持同步锁；退出先 drain，发送 Close 与等 actor 关闭共用 1.5s 超时，超时按 PID 杀进程树；进程回收不抹持久会话绑定 | R3-B |
| Lens → Chat pending external sends | `chat::external_send::ChatExternalSendMailbox` | enqueue / failed-open rollback / claim / renew / release / ack；同进程内至少一次交接，前端发送状态机按 request ID 去重；不保证跨进程恢复或恰好一次 | R3-B、R4 |
| 审批、会话授权、ask-user | `chat::interaction_state::ChatInteractionState` | 以原子行为完成/取消；不公开 sender/map；方法专属响应保留 | 前序批次，R3 保持 |
| Lens 图片/捕获/请求生命周期 | `lens::LensRuntimeState` | 图像身份、generation 和请求取消独立于 Chat；不因窗口卸载停止其他后台工作 | 前序批次，R3 保持 |
| MCP 会话与 schema 快照 | `mcp::McpRuntimeState` | session actor 及缓存生命周期仍由 MCP 管理；现有 session 句柄是领域 Interface | 前序批次，R3 保持 |
| 自动化执行 | `automation::AutomationRunState` | 句柄内部字段私有；运行转换与取消由 automation 管理 | 前序批次，R3 保持 |
| settings 值、epoch/revision、直连 HTTP OnceLock | `AppState` 私有设置/CAS 与客户端构造实现 | `settings_read` 只读 guard；写入经版本校验的行为入口；无外部裸 mutable 字段 | 已封装，保持 |
| `macos_ocr`、`offline_models`、`rapidocr`、`sub_agents` | 既有 OCR/model manager / `SubAgentManager` | 暴露的是封装了内部状态的领域句柄；不因本次整理另建可变副本 | 保留 |
| `usage_dir`、`http` | 应用组合资源 | 路径与 HTTP Client 依赖，非可任意写入的同步原语或领域索引 | 保留 |

## 验证与剩余范围

- 全仓枚举以原字段名及 `background_commands_handle` 为入口，迁移后裸 map/lock 访问仅存在于对应 owner；`AppState` 不再有 public 或 pub(crate) 的 Mutex/RwLock/Atomic 字段。返回只读 settings guard、异步执行许可与既有领域句柄是显式 Interface，不是原 map 写入权。
- 缓存已有 TTL/负缓存/LRU/容量并发测试保留，底层缓存测试随 Implementation 移到 discovery module；single-flight 改为同 key 等待、不同 key 通过、释放后可重试的行为验证。
- 新增验证涵盖 endpoint capability 隔离、后台任务终态元数据、跨会话清理、终止信号一次性和迟到完成、请求调试并发镜像顺序、两个前台槽隔离和并发 restore 单次消费、完整保存 gate 取消后可重获。Shell 原有后台进程/进程树/对话清理测试继续作为调用方回归。
- 请求调试已有磁盘镜像可能含用户请求内容；本批没有新增采集字段、改变默认开关或增加持久化范围，保留现有 header 脱敏与媒体裁剪。此前“完全不落盘”的注释不符合实现，已纠正；此处不声称实现了新的数据保留策略。
- model-probe key 表仍按原实现进程级保留；本次改变所有权，不引入新的容量/回收算法。`AppState` 不再为 Chat generation/reply/input、外部发现/常驻会话、后台作业或 provider failover/capability 提供一对一行为转发。调用方改走 `chat_runtime()` / `chat_interactions()` / `external_discovery()` / `external_live_sessions()` / `external_background_tasks()` / `background_commands_handle()` / `provider_runtime()`。跨域编排仍留在组合根：`cancel_chat_generation`（runtime + 子 agent）、`forget_chat_conversation_runtime`（runtime + interactions）、MCP 管理器装配、Settings 完整保存 permit。`state.rs` 门禁测试禁止把这些领域转发加回去。
- Windows 编译和 owner 测试不能替代 macOS 前台焦点、NSPanel、热键实机验收；该平台范围由整批交付明确记录。全量 cargo 验证由主任务在所有 R3 子任务完成后统一执行。
