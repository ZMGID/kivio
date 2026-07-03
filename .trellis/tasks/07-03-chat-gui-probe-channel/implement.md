# Implement：Chat GUI 无头测试通道（文件监听 probe）

前置：读 prd.md + design.md + research/。全程 `#[cfg(debug_assertions)]` 门控。

## 执行顺序

> 全部完成。实现与规划的差异（据实调整，均更优/低风险）：
> - Step 3 用 `probe: bool` 参数（而非 `ReplyHostMode` 枚举）接入；`run_agent_loop` 本就收 `&dyn AgentHost`，host 直接选 `&dyn` 局部，无需两臂重复调用；auto policy 复用既有 `arm.is_some()` 分支（改成 `|| probe`）。GUI 路径 probe=false 行为零变化（release 编译确认 probe 被 cfg 掉）。
> - Step 4 生成编排抽成 `commands::run_chat_probe`（复用生成核心 `complete_assistant_reply_inner`）；probe.rs 只做 watcher + IO。
> - **应用户要求改为不隔离**：probe 会话不删除，绑到固定复用的「Chat Probe」项目（根=cwd）、标题 `🔬 <prompt>`，保留在会话列表供观察调试。
> - 新增 `cwd` 请求字段（缺省=进程 cwd）让文件工具相对路径可解析（read/glob/grep 可测）。

### Step 1：probe 模块骨架 + 文件契约  ✅
- [x] `src-tauri/src/chat/probe.rs`（`#![cfg(debug_assertions)]`）+ `chat/mod.rs` 挂载。
- [x] `ProbeRequest {id?,prompt,provider?,model?,skillId,cwd?}` / `ProbeResult {id?,conversationId?,answer,toolCalls:[{name,arguments,status}],streamOutcome?,error?,finishedAt}`。
- [x] 单测：请求解析 + 结果序列化。

### Step 2：ProbeAgentHost（无头自动放行）  ✅
- [x] `commands.rs` 实现 `ProbeAgentHost`（挨着 ChatAgentHost）：emit no-op；审批/consent→true；ask_user→`cancelled_response()`；generation 方法复用 state 机制。

### Step 3：真实生成接入（复用 GUI 装配，仅换 host + 放行）  ✅
- [x] `complete_assistant_reply_inner` 加 `probe: bool`（两处现有调用传 false）；probe 分支 auto policy + `&dyn ProbeAgentHost`。

### Step 4：watcher + 请求处理 + 超时  ✅
- [x] `run_probe_watcher` 轮询 700ms + mtime 去抖 + 重命名 consumed；`handle_probe_request` 120s 超时兜底写 result；lib.rs `.setup` debug spawn。
- [x] `run_chat_probe`：固定「Chat Probe」项目绑 cwd → scratch 会话（保留）→ probe 生成 → 取 assistant 消息。

### Step 5：文档 + 全量检查  ✅
- [x] `docs/chat-probe.md` 用法文档。
- [x] `cargo check --lib --tests` 干净；`--release` 确认 probe 被 cfg 掉；`npm run typecheck && lint` 绿。
- [x] 集成自验（AC1/AC2/AC3/AC4）：真实 grok-composer 生成，捕获 `glob`(改名)/`read`(目录) success + 正确数字；自动放行无挂起；会话留存在「Chat Probe」项目。
- [x] AC5：release 不含 probe；会话/项目保留（按用户要求，非隔离）。
- [x] trellis-check 独立审查 PASS（7 项 + release 编译）。

## 回滚点
- probe.rs / ProbeAgentHost / run_chat_probe / lib.rs spawn 均可独立删除。
- 唯一碰 GUI 命令：`complete_assistant_reply_inner` 的 `probe` 参数（默认 false，行为不变）。

## 已知环境限制
- cargo test 二进制本机 0xC0000139 无法加载 → 纯函数逻辑靠编译通过 + 复核；行为验证靠运行中的 GUI app + 本 probe 通道本身（自举：probe 建好后即可用它做 AC2 验证）。
