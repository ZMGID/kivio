# Technical Design — 性能与资源生命周期修复

## 1. Design objective

为每类长生命周期资源建立明确的 owner、停止信号和终止等待点，并对进程级缓存采用有界策略；前端热路径则将结构性计算与高频流式内容更新分离。

统一生命周期形状：

```text
create/start → active work → cancel/timeout/idle/exit → signal stop
             → reject/drain dependents → await/reap → clear registry state
```

## 2. Ownership and shutdown rules

- 创建资源的一层负责保存 shutdown handle。
- `stop` 必须幂等，并先阻止新工作，再终止现有工作，最后清空可见状态。
- 子进程必须同时具备 terminate 与 reap；仅 drop handle 不算完成。
- 后台 task 必须有 cancellation signal，并在可控位置 await 或确认退出。
- 前端 Worker dispose 必须拒绝所有 pending promise，不能只终止 Worker 对象。
- 全局表只保存活资源；历史 key 使用 TTL/LRU/Weak 等有界机制。

## 3. PATH probe

`path_env.rs` 不再把已启动 child 移交给一个无法从超时方控制的阻塞闭包。实现采用共享 child owner 或可取消的异步等待结构，使超时方能够：

1. 请求 kill；
2. 等待 child 退出并消费 status/output；
3. join 等待线程；
4. 返回原有 fallback PATH。

实现需兼容当前 Rust/toolchain 和跨平台 shell 行为；测试使用短命/长命子进程验证完成与超时分支。

## 4. Pyodide worker lifecycle

前端 `pyodideClient` 引入单一 `resetWorker(reason)`：

- terminate 当前 Worker；
- 对 pending map 中每项 clearTimeout 并 reject；
- 清空 worker 引用和初始化 promise；
- 保证下一次调用按需创建新 Worker。

单请求超时采用与 Rust command 一致的总预算。若 Pyodide 无法安全中断单个 Python 执行，超时即重置整个 Worker，因此必须同时拒绝其他 pending 请求，避免 promise 永久悬挂。Worker `error`、`messageerror`、初始化失败和显式 dispose 复用同一路径。

Rust 侧只保留很小的传输/调度宽限，且不得早于前端执行超时到达；常量和注释明确两层预算关系，防止以后再次漂移。

## 5. macOS OCR helper lifecycle

Rust manager 保存 child、stdin/stdout 通道和最近活动状态，并增加：

- 请求前确认 helper 存活，失效则重建；
- 请求期间标记 busy，避免 idle shutdown；
- 后台空闲检查或每次请求后的延迟 shutdown generation；
- 显式 `shutdown()` 与 `Drop` 兜底：关闭 stdin、kill、wait/reap、清空 handles。

Swift helper 同时支持 EOF/显式 shutdown 后退出；可在协议层实现 idle deadline，避免 Rust 进程异常结束后 helper 永久常驻。双侧清理保持幂等，任一侧先退出都可恢复。

## 6. Preview server lifecycle

将 Preview server 的全局状态收敛为一个结构体，至少包含：

```text
server_port
reload_sender
shutdown_sender/token
server_task handle
```

listener accept loop 和每个 SSE heartbeat/select 分支监听同一个 cancellation token。`stop_all_previews()` 的顺序为：停止接受新连接 → 广播 shutdown → 停止预览进程 → await server task → 清空 sender/port/handles。启动函数在已有实例时复用或先完成旧实例清理，避免端口和 sender 串线。

## 7. Bounded cwd caches

将三个 cwd map 复用一个小型缓存抽象或一致的 helper：

```text
CacheEntry<T> { value, last_accessed }
BoundedCache<T> { entries, ttl, capacity }
```

- get：删除过期项，命中时刷新访问时间。
- insert：先清理过期项，再插入；超限时淘汰最久未访问项。
- 容量和 TTL 使用明确常量，测试中允许注入较小值/可控时钟。
- 外层锁持有时间只覆盖 map 操作，不包含 CLI I/O。

## 8. Reclaimable knowledge-base locks

锁表保存 `Weak<Mutex<...>>`，获取锁时在同一个表锁临界区内完成 upgrade-or-create，确保同一 `kb_id` 不会并发创建两个活锁。每次获取/删除库后顺便 `retain` 可升级条目；任务持有强 `Arc` 的期间锁不会消失，完成后允许释放。

测试覆盖：同 key 返回同一活锁、强引用释放后可回收、并发获取只有一个锁实例、不同 key 不互相阻塞。

## 9. MessageList hot path

把消息结构派生与流式内容派生分层：

- history/branch navigator 仅在消息身份、父子关系、分页边界或分支结构变化时重建；当前 assistant 文本追加不触发全量索引重算。
- 列表 item identity 保持稳定，活动流式消息只更新自己的内容 props。
- 自动贴底通过 `requestAnimationFrame` 合并，同一帧最多调用一次滚动；effect cleanup 取消旧 frame。
- 避免在 virtua 的测量/lifecycle 回调内同步触发 scroll/flush，必要时延迟到下一 animation frame。

测试以 render/derive 计数和滚动调用次数验证复杂度边界，并保留现有分支、工具消息、历史加载行为测试。

## 10. MCP warmup concurrency

启动预热改为有界 worker pool（优先复用 Tokio semaphore 或 `buffer_unordered(N)` 等现有依赖能力），并发常量保持较小。预热 task 仍与应用启动解耦，单 server 错误只记录摘要；所有 server 保留原有按需启动入口。若应用进入 shutdown，预热 future 应可自然取消或被统一 cleanup 覆盖。

## 11. App exit integration

复核现有 exit hook，将新增的 Preview shutdown、OCR shutdown 和其他 manager dispose 接入同一退出链。退出清理不得依赖窗口仍存在，也不得在主线程无限等待；必要时使用有界 timeout 后执行 kill/reap 兜底。

## 12. Observability and compatibility

- 仅记录资源类型、退出原因、耗时和错误类别，不记录用户内容。
- 新的 timeout/TTL/capacity/warmup limit 以命名常量集中定义。
- 默认用户行为保持兼容；资源退出后均支持透明按需重建。
- 清理失败记录 warning，但重复清理、已经退出和 channel 已关闭视为可接受状态。

## 13. Rollback points

- 每个资源类型独立提交逻辑边界，出现回归时可单独回退。
- 不改变持久化 schema，不需要用户数据迁移。
- MessageList 优化保留现有渲染结果与 public props，仅改变 memoization/scheduling。
