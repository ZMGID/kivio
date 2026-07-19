# Implementation Plan — 性能与资源生命周期修复

## Phase A — 建立生命周期原语与针对性测试

- [x] 确认现有 app exit、Preview、OCR、Pyodide、MCP manager 的 owner 和调用路径。
- [x] 为 kill/reap、cancel/await、reject-all、TTL/LRU、Weak lock 等行为先补最小失败测试或测试 seam。
- [x] 集中定义 timeout、idle、capacity 和 warmup concurrency 常量，避免 magic number 分散。

## Phase B — 子进程与 Worker 回收

- [x] 修复 `src-tauri/src/path_env.rs`：超时 kill/reap child 并 join 等待线程。
- [x] 修复 `src/chat/pyodideClient.ts`：统一 reset/dispose，清空并拒绝全部 pending，允许透明重建。
- [x] 对齐 `src-tauri/src/mcp/registry.rs` 与前端 Pyodide timeout 预算。
- [x] 增加 PATH timeout、Worker timeout/error/dispose/multi-pending 测试。

Validation:

```bash
cargo test --manifest-path src-tauri/Cargo.toml path_env
npm test -- --run pyodide
```

## Phase C — OCR helper 生命周期

- [x] 为 Rust OCR manager 增加显式 shutdown、Drop/reap 兜底和退出后重启。
- [x] 为 Swift helper 增加 EOF/shutdown/idle 退出机制。
- [x] 将 OCR shutdown 接入应用退出路径。
- [x] 增加协议/manager 测试；macOS 上构建 helper 并做 idle/restart smoke test。

## Phase D — Preview server 完整停机

- [x] 给 listener/server task 增加 cancellation handle。
- [x] 让 SSE heartbeat 和连接处理监听 shutdown。
- [x] 扩展 `stop_all_previews()`，停止进程和 server，并清空 `RELOAD_TX/server_port`。
- [x] 增加幂等 start/stop/restart 与端口释放测试。

## Phase E — 有界缓存与可回收锁表

- [x] 将 `src-tauri/src/state.rs` 三个 cwd 缓存统一改为带 TTL/capacity 的实现。
- [x] 为缓存增加过期、LRU/旧项淘汰、刷新、并发测试。
- [x] 将知识库 ingest 锁表改为 Weak upgrade-or-create，并在安全时机清扫。
- [x] 增加同库串行、不同库并行、弱引用回收和并发单实例测试。

## Phase F — MessageList 流式热路径

- [x] 用 CodeGraph 跟踪 `MessageList` 的 items、navigator 和 scroll 调用链，确定稳定依赖边界。
- [x] 拆分 memoization，使流式文本追加不触发结构导航全量重建。
- [x] 将贴底滚动合并到 requestAnimationFrame，并正确 cleanup。
- [x] 调整测试以断言长列表更新的派生/滚动次数，并消除相关 `flushSync` warning。

Validation:

```bash
npm test -- --run MessageList
npm run typecheck
npm run lint
```

## Phase G — MCP 预热限流与退出集成

- [x] 将启动时 MCP warmup 改为固定小并发上限。
- [x] 验证预热失败不阻塞启动，未预热/失败 server 仍可按需连接。
- [x] 复核 app exit 清理顺序，确保 Preview、OCR、MCP、CLI、background commands 均被覆盖。
- [x] 添加并发上限与错误隔离测试或确定性 instrumentation 测试。

## Phase H — 完整质量门禁

- [x] 运行格式化与针对性测试，修复新增 warning。
- [x] 运行全部前端 lint、typecheck、tests。
- [x] 运行全部 Rust lib tests。
- [x] macOS 上构建/测试 OCR Swift helper。
- [x] 检查 `git diff`，确认用户原有修改未被覆盖。
- [x] 使用 `trellis-check` 做规范、测试、数据流和一致性复核。

## Verification Results — 2026-07-19

- `npm run lint` — passed.
- `npm run typecheck` — passed.
- `npm test -- --run` — 50 files, 295 tests passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib` — 1493 passed, 12 ignored.
- `cargo check --manifest-path src-tauri/Cargo.toml` — passed.
- `swift build --package-path src-tauri/swift/kivio-ocr-helper` — passed.
- Swift helper shutdown/restart protocol smoke — two clean launches and exits passed.
- `git diff --check` and task-scoped `rustfmt --check` — passed.
- Known baseline warning only: `src-tauri/src/plugins/install.rs::bundle_summary` is unused.

Final validation:

```bash
npm run lint
npm run typecheck
npm test -- --run
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

## Elevated-risk files

- `src-tauri/src/path_env.rs` — 跨平台子进程终止与阻塞线程 join。
- `src-tauri/src/mcp/registry.rs`、`src/chat/pyodideClient.ts` — 跨 Rust/Worker timeout 契约。
- `src-tauri/src/macos_ocr.rs`、`src-tauri/swift/kivio-ocr-helper/Sources/main.swift` — 双进程协议与 macOS 专属生命周期。
- `src-tauri/src/plugins/preview.rs` — 全局 server 状态、listener 和 SSE 并发任务。
- `src-tauri/src/state.rs` — 多处共享缓存语义。
- `src-tauri/src/chat/knowledge_base/ingest.rs` — 同库并发串行保证。
- `src/chat/MessageList.tsx` — 虚拟列表与流式渲染热路径。
- `src-tauri/src/lib.rs` — 应用启动和退出集成。

## Planning gate

- [x] 全部 8 项缺陷已有源码锚点和可测试验收标准。
- [x] 已确认正常的窗口/overlay/监听器不纳入无差别重写。
- [x] PRD 已完成 convergence pass，无重复临时结论或未决范围问题。
- [x] 用户已明确要求创建 Trellis 并直接实施全部修复。
