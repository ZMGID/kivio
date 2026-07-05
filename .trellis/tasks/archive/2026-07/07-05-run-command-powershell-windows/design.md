# 技术设计：run_command 在 Windows 改用 PowerShell

## 涉及文件

- `src-tauri/src/native_tools/shell.rs` — 执行 shell 构建（前台 + 后台）、测试。
- `src-tauri/src/chat/agent/prepare.rs` — 系统提示词 shell 名与语法引导（Windows 分支）。

## 参考实现（opencode，已拉取分析）

- `packages/core/src/shell.ts`
  - `win()`：候选顺序 `[which("pwsh"), which("powershell"), gitbash(), COMSPEC||cmd.exe]`，去重后取第一个可用。→ **pwsh 优先，powershell 次之**。
  - `args()`：`if ps(file) return ["-NoProfile", "-Command", command]`；`cmd` → `["/c", command]`。
  - `killTree()`：Windows `taskkill /pid <pid> /f /t`。
- `packages/opencode/src/tool/shell.ts` `cmd()`：`ChildProcess.make(shell, ["-NoLogo","-NoProfile","-NonInteractive","-Command", command], {cwd, env, stdin:"ignore", detached:false})`。命令作单个参数，**不用 -EncodedCommand**。
- `prompt.ts`：按所选 shell 注入对应语法说明（pwsh 支持 `&&`；5.1 用 `; if ($?){}`；都强调全名 cmdlet、`$env:`）。Kivio 走精简版（见下）。

## 设计

### 1. shell 选择与命令构建（shell.rs）

当前前台 `run_shell_command`（L674）和后台 `run_shell_command_background`（L414）各自内联了同一段
`#[cfg(windows)] Command::new("cmd"); raw_arg("/C"); raw_arg(command)`。抽出共享构建函数，避免两处漂移：

```rust
/// Windows: 选定的 PowerShell 可执行名（缓存）。优先 pwsh(7+)，回退 powershell(5.1)。
#[cfg(target_os = "windows")]
fn windows_powershell_exe() -> &'static str {
    use std::sync::OnceLock;
    static SHELL: OnceLock<&'static str> = OnceLock::new();
    SHELL.get_or_init(|| if pwsh_on_path() { "pwsh" } else { "powershell" })
}

#[cfg(target_os = "windows")]
fn pwsh_on_path() -> bool {
    // 手动扫 PATH，不引新依赖。PATHEXT 固定含 .exe。
    std::env::var_os("PATH").map_or(false, |paths| {
        std::env::split_paths(&paths).any(|dir| dir.join("pwsh.exe").is_file())
    })
}

/// UTF-8 输出编码前缀：让 5.1 的 stdout 也走 UTF-8（pwsh 7 已默认 UTF-8，前缀无害）。
/// try/catch 兜底：极端无控制台环境下设 OutputEncoding 抛错时不影响主命令。
#[cfg(target_os = "windows")]
fn wrap_ps_command(command: &str) -> String {
    format!(
        "try {{ [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 }} catch {{}}; {command}"
    )
}

/// 前台/后台共用：按平台构建执行 shell 的 tokio Command（不含 stdio/env/flags 设置）。
fn build_shell_command(command: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut c = Command::new(windows_powershell_exe());
        // raw_arg 而非 args()：绕开 MSVC 对参数的转义，命令内部引号原样到达 PowerShell
        //（沿用既有 cmd 路径的理由）。-Command 取其后整段为脚本。
        c.raw_arg("-NoLogo");
        c.raw_arg("-NoProfile");
        c.raw_arg("-NonInteractive");
        c.raw_arg("-Command");
        c.raw_arg(wrap_ps_command(command));
        c
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    }
}
```

两处调用点改为 `let mut cmd = build_shell_command(&command);`，其余（`current_dir`/`stdin(null)`/`stdout`/`stderr`/`apply_shell_tool_env`/creation_flags/`kill_on_drop`/pre_exec）保持不变。

- 前台：保留 `creation_flags(CREATE_NO_WINDOW)`、`kill_on_drop(true)`。
- 后台：保留 `creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)`、`kill_on_drop(false)`、waiter 逻辑、`taskkill /T /F` 杀树（`kill_process_group` 不变）。

### 2. 提示词同步（prepare.rs）

- L778-784：Windows 分支 `("Windows", "cmd.exe")` → `("Windows", "PowerShell")`（不做 pwsh/5.1 细分——对模型而言都是 PowerShell，写 PS 语法即可，保持 Kivio 精简风格）。
- L843（中）/L869（英）的语法引导子句替换：
  - 中：把 “命令语法须匹配该 shell（Windows 用 `%VAR%`、`dir`、`\\`；Unix 用 `$VAR`、`ls`、`/`）” 改为
    “命令语法须匹配该 shell（Windows PowerShell：用全名 cmdlet 如 `Get-ChildItem`/`Get-Content`、环境变量 `$env:VAR`、多条命令用 `;` 串联，别用已废弃的 `wmic`，也别对整盘做 `-Recurse` 扫描；Unix：`$VAR`、`ls`、`/`）”。
  - 英：对应改为 PowerShell 引导（full-name cmdlets、`$env:VAR`、chain with `;`、avoid `wmic` and whole-disk `-Recurse`）。
- 其余（fresh process/cwd、inline 引号脆弱、失败不重跑等）保持。

### 3. 数据流 / 契约影响

- 工具入参 schema（`command`/`cwd`/`timeout_ms`/`background`/…）不变；对模型可见契约仅"shell 语义从 cmd 变 PowerShell"。
- 退出码语义变化（可接受，与 opencode 一致）：cmd 下非零退出→Err 的判断保留（`output.status_code != 0`）。PowerShell 进程退出码由最后一条命令决定——原生 exe 传播 `$LASTEXITCODE`，cmdlet 非终止错误不置非零。即 cmdlet 报错不再必然 Err；这是 PowerShell 固有语义，opencode 亦如此。
- 编码：stdout/stderr 仍 `from_utf8_lossy`。前缀保证 5.1 输出 UTF-8 字节，中文可解。

## 兼容性 / 风险

- **历史对话**：命令不回放，风险极低。
- **`&&` 串联**：5.1 不支持；`normalize_run_command` 已剥离前导 `cd x &&`；提示词引导 `;`。模型若仍写 `a && b`，5.1 会报解析错（一次失败后按提示改）——可接受，且 pwsh 环境下 `&&` 正常。
- **无控制台设 OutputEncoding**：CREATE_NO_WINDOW 仍分配（隐藏）控制台，正常可设；try/catch 兜底防御。
- **超时杀进程**：Windows 前台超时依赖 `kill_on_drop(true)` 杀 PowerShell 进程本身（不杀孙进程）——与现状 cmd 相同，不劣化，暂不扩展。
- **denylist**：面向 unix（`rm -rf /` 等），Windows 下基本不触发；保持不变（非目标）。

## 测试计划（shell.rs `#[cfg(target_os="windows")]`）

1. 改造 `windows_run_command_preserves_embedded_quotes`：`python -c "print(40 + 2)"` 经 PowerShell 仍得 `42`（python 不在 PATH 时跳过，逻辑保留）。
2. 新增 `windows_run_command_executes_via_powershell`：`Write-Output (1+1)` → stdout 含 `2` 且 exit 0（cmd 无法解析该语法，证明确实走 PowerShell）。
3. 新增 `windows_run_command_outputs_utf8`：`Write-Output '你好'` → stdout 含 `你好`（守护编码前缀）。
4. 后台测试 `kill_background_marks_killed_and_terminal_is_noop` 的 Windows 长命令由 `ping -n 30 127.0.0.1 > NUL` 改为 `Start-Sleep -Seconds 30`（纯 PowerShell，且不产生 `NUL` 杂散文件）。其余后台测试（tracked/poll/incremental/kill_all）跨平台命令 `echo <token>` 在 PowerShell 下同样有效，无需改。

验证命令：Windows 经 `scripts/win-cargo-test.ps1`（见 [[windows-rust-test-manifest]]）跑 `native_tools::shell` 相关测试，对照 [[windows-cargo-lib-preexisting-failures]] 基线判定无新增失败。
