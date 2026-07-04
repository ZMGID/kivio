# 执行计划：run_command 在 Windows 改用 PowerShell

## 顺序清单

### 步骤 1 — shell.rs：抽共享命令构建 + PowerShell 切换
- [ ] 新增 `#[cfg(target_os="windows")]` 辅助：`pwsh_on_path()`、`windows_powershell_exe()`（OnceLock 缓存）、`wrap_ps_command()`。
- [ ] 新增跨平台 `build_shell_command(command: &str) -> Command`：Windows 走 `<pwsh|powershell> -NoLogo -NoProfile -NonInteractive -Command <wrapped>`（全 `raw_arg`）；非 Windows 走 `sh -c`。
- [ ] `run_shell_command`（前台）改用 `build_shell_command`，保留 stdio/env/`CREATE_NO_WINDOW`/`kill_on_drop(true)`/macos setsid。
- [ ] `run_shell_command_background`（后台）改用 `build_shell_command`，保留 stdio→log/env/`CREATE_NO_WINDOW|CREATE_NEW_PROCESS_GROUP`/`kill_on_drop(false)`/waiter/unix setsid。
- [ ] 删除两处内联的 `Command::new("cmd") + raw_arg("/C")` 重复块。

### 步骤 2 — shell.rs：测试
- [ ] 改造 `windows_run_command_preserves_embedded_quotes`（PowerShell 下 `python -c "print(40 + 2)"` → 42；python 缺失跳过）。
- [ ] 新增 `windows_run_command_executes_via_powershell`（`Write-Output (1+1)` → `2`, exit 0）。
- [ ] 新增 `windows_run_command_outputs_utf8`（`Write-Output '你好'` → 含 `你好`）。
- [ ] 后台测试 Windows 长命令改 `Start-Sleep -Seconds 30`。
- [ ] `apply_shell_tool_env_injects_output_hygiene` 里 `Command::new("cmd")` 仅用于构建空 Command 检查 env，不受影响（保留或改中性名，二选一，优先保留以缩小 diff）。

### 步骤 3 — prepare.rs：提示词
- [ ] Windows 元组 `("Windows","cmd.exe")` → `("Windows","PowerShell")`。
- [ ] 中文语法引导子句改 PowerShell 版（cmdlet 全名 / `$env:VAR` / `;` 串联 / 禁 `wmic` / 禁整盘 `-Recurse`）。
- [ ] 英文语法引导子句同步改 PowerShell 版。
- [ ] 若有断言 `cmd.exe`/`%VAR%` 的 prepare 测试，更新预期。

### 步骤 4 — 校验
- [ ] `cargo build`（或 `cargo check`）Windows 编译通过。
- [ ] `scripts/win-cargo-test.ps1` 跑 `native_tools::shell`（及 prepare 相关）测试，对照基线无新增失败。
- [ ] 运行时冒烟（可选，dev 或 chat-probe）：让 chat 跑"查 C 盘空间/已安装软件"，确认走 `Get-PSDrive`/`Get-ItemProperty` 类命令且秒回，不再空转。

## 验证命令
```
# Windows 测试（见 windows-rust-test-manifest 记忆）
powershell -ExecutionPolicy Bypass -File scripts/win-cargo-test.ps1
# 或聚焦
cargo test --manifest-path src-tauri/Cargo.toml native_tools::shell
```

## 评审门 / 回滚点
- 步骤 1 后可 `cargo check` 确认编译（回滚点 A：仅 shell.rs 改动，git restore 单文件即可还原）。
- 步骤 3 为纯文案，独立可回滚（回滚点 B）。
- 任一步测试出现"新增"失败（非基线）即停下排查，不硬推。

## 不做
- 不加用户可配 shell、不加 PowerShell denylist、不改 macOS/Linux、不动前端。
