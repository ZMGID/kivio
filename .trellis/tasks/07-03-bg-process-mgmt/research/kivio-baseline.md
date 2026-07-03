# Kivio 现状基线（对比锚点）

> 本文件是 Kivio **当前**后台/长驻进程实现的基线，供与 opencode/codex/gemini-cli/aider/claude-code 的调研结果对比。全部指向 `src-tauri/src/native_tools/shell.rs`。

## 1. 后台 / 长驻进程模型
- `run_command` 工具，参数 `background: bool`。未显式传时由 `is_long_running_dev_command(command)` 自动判定（shell.rs:135, :301-319；命中 `npm run dev` / `vite` / `webpack-dev-server` 等前缀 → 自动转后台）。
- 后台路径 `run_shell_command_background`（shell.rs:405）：
  - stdout+stderr 合并写入 temp 日志 `kivio-bgcmd-<job_id>.log`（BG_CMD_LOG_PREFIX, shell.rs:323, :411）。
  - 注册进 `AppState.background_commands`（job_id → pid/log/status/command/cwd/started_at）。
  - 一个 waiter task 持有 Child、等它退出并更新状态；`kill_on_drop(false)` → **跨轮存活**，只由 `kill_background` 或 app 退出清扫杀掉。
- 轮询：`bash_output`（shell.rs:537）——带 job_id 按 offset 增量读日志 + 状态；不带 job_id 时列出所有 job（= 旧 list_background，shell.rs:588）。
- 停止：`kill_background`（shell.rs:617）——Windows `taskkill /T /F <pid>`（进程树），unix `kill(-pgid)` SIGTERM→SIGKILL。

## 2. Windows 控制台窗口抑制
- 前台（shell.rs:694）：`creation_flags(CREATE_NO_WINDOW)` —— 单独用，隐藏控制台且被子孙继承，无弹窗。
- **后台（shell.rs:445-447）：`CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`** —— **这是 bug**：按 MSDN，`CREATE_NO_WINDOW` 与 `DETACHED_PROCESS` 同设时前者被忽略；`DETACHED_PROCESS` 让 cmd.exe 无控制台，其控制台子孙进程（npm/tsup/vite/electron）因无可继承控制台而各自新建**可见**控制台窗口 → 用户看到的黑框。
- CJK/引号：Windows 用 `cmd /C` + `raw_arg` 绕开 MSVC 转义（shell.rs:424-428, :667-673）；unix 用 `sh -c` + `setsid`（新会话，便于 killpg）。

## 3. 就绪 / 运行状态回传给 LLM（当前最弱的一环）
- `bash_output` 的 status 只有四态：`running` / `exited(code)` / `killed` / `error`（shell.rs:563-571）。
- 对 dev server：状态**永远是 `running`**、日志一直在流，**没有任何"已就绪/健康/URL"的语义信号**。模型只能从原始日志文本自行猜是否起来了。
- 启动即返回一句 banner（shell.rs:530）："Started in the background… Poll with bash_output… **Do not start the same dev server twice.**" —— 纯文字提示，无强制。
- 无宽限期：启动**立即**返回，第一次返回里没有初始输出/就绪判定 → 模型看到空的"去 poll 吧"，容易焦虑重试。

## 4. 防重复启动 / 幂等
- **无**。仅靠 banner 里一句"Do not start the same dev server twice"。注册表不按 command/cwd/port 去重；模型再调一次 `run_command` 就会再 spawn 一个 → 端口冲突 / electronmon 反复重启 / 闭环。

## 5. dev server 特殊处理
- 有"是否长驻 dev 命令"的自动后台化启发式（`is_long_running_dev_command`, shell.rs:301），但**仅用于决定是否后台**；没有端口探测、没有 URL 提取、没有就绪检测、没有去重。

## 用户报告的现象 → 归因
- **弹黑框** → §2 的 DETACHED_PROCESS bug。
- **反复重启 / 看不懂状态** → §3（状态不可读、无就绪信号、无初始输出）+ §4（无防重复）共同导致：模型看不清"起来了"，又没东西拦它重试 → 反复拉起 → 抢端口 → 更乱。

## 待调研工具需回答（用于设计取舍）
- 是否有"就绪检测/状态语义"而非裸 running？（§3 的主要缺口）
- 是否幂等/防重复？（§4）
- Windows 隐藏窗口的正确标志组合？（§2 印证）
- 后台模型（start/poll/stop/registry）与 Kivio 的异同。
