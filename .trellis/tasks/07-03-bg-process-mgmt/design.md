# Design — 后台进程 / dev-server 运行状态管理重构

> 全部改动锚定 `src-tauri/src/native_tools/shell.rs`（后台路径 `run_shell_command_background` :405、前台 :660、`bash_output` :537、flags :445-447/:694、env 注入 `apply_shell_tool_env` :51）。契约参考 `research/kivio-baseline.md`，业界对照见 `research/{opencode,codex,gemini-cli,others}.md`。

## 总体思路
保留现有"后台 job 注册表 + 文件日志 + bash_output 偏移轮询 + kill_background"骨架不动，只在**四处**做精修（全是"提升可见性"，不含控制/限制）：spawn 标志（消弹窗）、启动 yield window（首返回带状态）、就绪可读性提示、环境净化。均向后兼容。**明确不做**：防重复启动（控制类）、内置终端 / PTY。

---

## D1. Windows 弹窗修复（G1）
**改**：`run_shell_command_background` 的 creation flags（shell.rs:445-447）
```
- cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
+ cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
```
**依据**：MSDN — `CREATE_NO_WINDOW` 与 `DETACHED_PROCESS` 同设时前者被忽略。去掉 `DETACHED_PROCESS` 后，cmd.exe 拿到**隐藏控制台**，其 npm/vite/electron 子孙继承该隐藏控制台 → 全程无可见窗口。stdout/stderr 仍通过继承的文件句柄写日志（捕获不变）。`CREATE_NEW_PROCESS_GROUP` 保留 → `taskkill /T /F <pid>` 仍按树杀。
**为何不动前台**：前台 :694 只用 `CREATE_NO_WINDOW`，本就正确。
**存活性**：跨轮存活靠 `kill_on_drop(false)` + waiter 持有 Child，与 `DETACHED_PROCESS` 无关，去掉无副作用。

## D2. 启动 yield window（G2 核心）
**改**：`run_shell_command_background` 在 spawn 之后、返回之前，加入一个有界等待窗口。
- 常量：`const BG_STARTUP_GRACE: Duration = Duration::from_millis(1500)`（codex 250ms–30s、gemini 200ms 之间取中值；后续可调）。
- 逻辑（不阻塞其它请求，用 `tokio::time::timeout` 包住"等待进程退出"）：
  1. spawn 后，`timeout(BG_STARTUP_GRACE, child_wait)`：
     - **窗口内退出** → 判定这不是长驻进程 / 启动即失败：读全量日志，按**前台结果形态**返回 `exit_code + output`（不注册为后台 job，或注册后立即标 Exited）。这直接消灭"启动失败却报 background: true"的误导（对齐 gemini 200ms 抓崩溃、codex yield window 语义）。
     - **窗口到点仍在跑** → 走现有后台注册流程，但**首次返回体里带上这段初始输出**（读当前日志）+ 明确状态行。
- 首次返回文案（替换 shell.rs:530 的 banner）示意：
  ```
  background: true
  job_id: <id>
  pid: <pid>
  cwd: <cwd>
  status: running (仍在运行，已捕获前 1.5s 输出)
  --- initial output ---
  <前 1.5s 的日志尾部, 截断到 N 行/字节>
  ---
  它在后台常驻并跨轮存活。用 bash_output(job_id) 增量看输出、kill_background(job_id) 停止。
  若上面已出现服务就绪迹象（如 ready / Local: http://…），说明已起来，请勿重复启动。
  ```
**注意**：等待"进程退出"需要能在不消费 waiter 所有权的前提下探测。实现选型见 implement.md（用一个可 clone 的退出通知，或让 waiter 先跑一小段再交接）。这是本设计的**主要实现风险点**，须在 implement 阶段先落地探测机制再接线。

## D3. 轻量就绪提示（G2 增强，克制）
**改**：`bash_output`（shell.rs:563-583）在 `running` 状态下，额外扫描日志尾部常见就绪 marker，命中则在 status 行追加提示（**只是提示，不是新状态**）：
- marker 集（大小写不敏感，正则/包含）：`ready in`、`Local:`、`listening on`、`compiled successfully`、`watching for changes`、`waiting for a change`、`http://localhost`、`ready - started server`。
- 输出：`status: running (likely ready ✓ — matched "Local:")`。
**依据**：业界无人做就绪检测（4 份 research 一致），所以只做"提示"不做"状态机"，零误导成本；帮模型识别"起来了、别重启"。可作为 D2 的补充，若时间紧可后置。

## D4. 环境净化（G4）
**改**：`apply_shell_tool_env`（shell.rs:51）对**后台与前台**子进程统一注入（不覆盖用户已在 settings 里显式设的同名 env）：
- `TERM=dumb`、`NO_COLOR=1`、`CI=1`、`PAGER=cat`、`FORCE_COLOR=0`。
**依据**：codex 对所有 unified-exec 子进程注入同类变量（`research/codex.md`）。减少 ANSI 彩色码/交互分页/进度条刷屏 → 日志更干净、模型更易读，也降低 TTY 驱动的异常重启概率。
**风险**：极少数命令依赖彩色输出；但对"给模型读"的场景利大于弊，且用户显式 env 可覆盖。

---

## 数据流 / 契约（不变）
- job 注册表：`AppState.background_commands: job_id → {pid, log_path, status, command, cwd, started_at}`。本次不新增查询/写操作。
- 日志：`kivio-bgcmd-<job_id>.log`，格式不变；D2 首次返回只是**多读一次**当前日志尾部。
- `bash_output` 偏移轮询、`kill_background`、无 job_id 列表：不变（D3 仅在 running 分支追加一行提示文本）。
- `run_command` 工具 schema：不变（不加/删参数；yield window、env 都是内部行为）。

## 兼容与回滚
- 每个 D 项独立、可单独回滚（D1 一行、D2 一段、D3 一段、D4 一行 env 列表）。
- headless kivio-code（无 AppState）路径：D2 若无注册表则退化为"等待窗口后按 legacy fire-and-forget 返回 + 初始输出"。

## 取舍记录
- **不做防重复启动 / 任何控制类限制**：用户定调靠模型+可见性；且业界（4 份调研）无先例。模型来回拉扯的根因是"看不清状态"，由 D2/D3 解决。
- **不做内置终端 / PTY**：用户已否；现有 opencode/codex 交互体验够用，不引入此复杂度。
- **不做完整就绪状态机 / 端口探测 / URL 结构化**：4 份调研一致无先例，重型实现收益不明（非目标）。D3 用轻量 marker 提示替代，够用。
- **不做 completion-injection**：gemini 有、但需接 Kivio 事件/对话注入链路，属更大改动，列后续增强。
- **不 ConPTY**：与文件日志捕获模型冲突，本次不引入。
