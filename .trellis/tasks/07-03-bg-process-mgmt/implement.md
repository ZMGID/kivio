# Implement — 后台进程 / dev-server 运行状态管理重构

> 前置：本轮仅规划。实现须待 `task.py start`（in_progress）后再动手。Windows 测试一律用
> `scripts/win-cargo-test.ps1 --lib <filter>`（直接 `cargo test` 会 0xC0000139）。
> 全部改动锚定 `src-tauri/src/native_tools/shell.rs`。
> **原则：只提升可见性，不加控制/限制**（防重复启动、内置终端已明确排除）。按下列顺序做，每步独立可回滚。

## 步骤 0：准备
- [ ] `task.py start`（评审通过后）。
- [ ] 通读 `run_shell_command_background`（:405）、`bash_output`（:537）、`apply_shell_tool_env`（:51）。

## 步骤 1（D1）：Windows 弹窗修复 — 最低风险，先做
- [ ] shell.rs:447 去掉 `DETACHED_PROCESS`，改 `creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)`；更新注释说明 MSDN 冲突。
- [ ] 校验：`win-cargo-test.ps1 --lib native_tools::shell`（既有 background/kill 测试不回归）。
- 回滚点：单行还原。

## 步骤 2（D4）：环境净化
- [ ] `apply_shell_tool_env`（:51）在应用 settings env 之前，先注入默认 `TERM=dumb`/`NO_COLOR=1`/`CI=1`/`PAGER=cat`/`FORCE_COLOR=0`（用户 settings 里同名 env 可覆盖 → 后写覆盖先写）。
- [ ] 新增单测：构造 Command，断言这些 env 存在；断言用户显式同名 env 覆盖默认值。
- [ ] 校验：`win-cargo-test.ps1 --lib native_tools::shell`。
- 回滚点：删除注入块。

## 步骤 3（D2）：启动 yield window — 核心、风险最高，放在有测试兜底之后
- [ ] 常量 `BG_STARTUP_GRACE = Duration::from_millis(1500)`。
- [ ] `run_shell_command_background` spawn 后：`match tokio::time::timeout(BG_STARTUP_GRACE, child.wait()).await`
  - `Ok(Ok(status))`（窗口内退出）：读全量日志，按**前台结果形态**返回 `status: exited / exit_code / output`（不注册为长驻，或注册后立即标 Exited 再返回）。`child.wait()` 借用 `&mut child`、不消费所有权。
  - `Err(_)`（超时仍在跑）：读当前日志尾部作为"初始输出"，走现有注册 + waiter 流程（把 `child` move 进 waiter task 继续 `wait()`），首次返回体带上初始输出 + `status: running`。
  - `Ok(Err(e))`：spawn/wait 出错 → 返回 error。
- [ ] 更新首次返回 banner（:530）为带 initial output + 状态的形态（见 design.md D2）。
- [ ] 验证 tokio `Child::wait()` 在 timeout 丢弃后不杀子进程（`kill_on_drop(false)` 已保证），再次 move 进 waiter `wait()` 正常。
- [ ] 新增单测：
  - 快退命令（`exit 3` / `sh -c 'echo boom; exit 1'`）→ 首次返回含 exit_code、非 "background: true 仍在跑"。
  - 长驻命令（`sleep 5` 类）→ 首次返回含 job_id + status running + 初始输出；随后 `bash_output` 仍能增量读、`kill_background` 能停。
- [ ] 校验：`win-cargo-test.ps1 --lib native_tools::shell`（用 test-util 暂停时钟或短 grace 避免测试慢）。
- 回滚点：把 spawn 后逻辑还原为"立即注册并返回"。

## 步骤 4（D3）：轻量就绪提示（可选，可后置）
- [ ] `bash_output`（:563 running 分支）：扫描日志尾部 marker 集（design.md D3），命中则 status 行追加 `(likely ready ✓ — matched "…")`。仅提示、不改状态枚举、不限制行为。
- [ ] 新增单测：日志含 `Local: http://localhost:5173` → 输出含 likely ready；不含 → 不追加。
- [ ] 校验：`win-cargo-test.ps1 --lib native_tools::shell`。
- 回滚点：删 marker 扫描块。

## 步骤 5：全量校验 + 冒烟
- [ ] `win-cargo-test.ps1 --lib native_tools::shell` 全绿；`cargo build` 确认主程序编译。
- [ ] 手动冒烟（AC1/AC2）：在一个临时 vite/electron 项目里让 Kivio agent 跑 `npm run dev`：
  - 无黑框弹出；
  - 首次工具返回即能看到 vite ready / Local URL + status running。
- [ ] 更新 spec（3.3）：在 tauri/shell 相关 spec 记录后台进程语义（yield window / env / flags）。

## 校验命令速查
- 单测：`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/win-cargo-test.ps1 --lib native_tools::shell`
- 编译：`cargo build --manifest-path src-tauri/Cargo.toml --no-default-features`

## 风险与缓解
- **R1（主要）**：yield window 的进程退出探测与 waiter 所有权交接。缓解：用 `timeout(grace, child.wait())` 借用式探测（不消费 Child），超时后再 move 进 waiter；步骤 3 前先有步骤 1-2 的测试兜底。
- **R2**：grace=1.5s 拖慢"快退命令"外的正常后台返回。可接受（一次性 1.5s），必要时按命令类型调小；单测用短 grace/暂停时钟。
- **R3**：env 净化影响个别依赖彩色/TTY 的命令。缓解：用户 settings env 可覆盖；只注入不覆盖。
