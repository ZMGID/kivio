# run_command 在 Windows 改用 PowerShell 执行

## 背景 / 问题

同一个"查 C 盘空间和已安装软件"的任务，opencode 秒完成，Kivio 却长时间空转最终被用户手动停止。

对比抓包（`C:\Users\11028\Downloads\trace_c72cfbee.html`）与 Kivio 失败对话记录，根因定位：

- **Kivio**：`run_command` 在 Windows 走 `cmd.exe`（`shell.rs` 编译期 cfg），系统提示词还告诉模型"Windows 用 `%VAR%`、`dir`、`\\`"。模型于是先用 cmd 时代工具 `wmic`（新版 Windows 已移除/挂起）、`dir /S`、整盘 `Get-ChildItem -Recurse` 等 → 卡死、空转。
- **opencode**：其 bash 工具在 Windows 直接走 **PowerShell**（抓包里全是未包裹的原生 PowerShell cmdlet：`Get-PSDrive C | Select-Object Used,Free`、`Get-ItemProperty HKLM:\...\Uninstall\*`），模型自然写现代 PowerShell 一行命令，秒出结果。

结论：最根本、也是与 opencode 对齐的修复 = 让 Kivio 的 `run_command` 在 Windows 直接用 PowerShell 执行，并同步系统提示词的 shell 名与语法引导。（对照实现：opencode `packages/core/src/shell.ts` 的 `win()`/`args()` 与 `packages/opencode/src/tool/shell.ts` 的 `cmd()`。）

## 目标

Windows 上 `run_command`（前台 + 后台）改用 PowerShell 执行，并让系统提示词如实反映 PowerShell，从而消除模型走 cmd 老工具导致的卡死/空转，行为向 opencode 看齐。macOS/Linux 行为完全不变。

## 需求

1. **执行 shell 切换（Windows）**：前台 `run_shell_command` 与后台 `run_shell_command_background` 都从 `cmd /C` 改为 PowerShell 调用。
   - shell 选择：优先 `pwsh`（PowerShell 7+，若在 PATH），否则回退 `powershell.exe`（Windows PowerShell 5.1，系统必带）。检测结果进程内缓存。
   - 调用形态对齐 opencode：`<shell> -NoLogo -NoProfile -NonInteractive -Command <命令>`，命令作为原样参数传入（沿用现有 `raw_arg` 手法，保证内部引号原样到达，见既有回归测试 `windows_run_command_preserves_embedded_quotes`）。
2. **UTF-8 输出编码**：确保 PowerShell（尤其 5.1，默认非 UTF-8）的 stdout 以 UTF-8 输出，中文/非 ASCII 不乱码。
3. **提示词同步**（`prepare.rs`）：Windows 分支的 `shell_name` 由 `cmd.exe` 改为 `PowerShell`；把"Windows 用 `%VAR%`、`dir`、`\\`"这段语法引导改为 PowerShell 引导（用全名 cmdlet、`$env:VAR`、用 `;` 串联、避免 `wmic`、避免整盘 `-Recurse` 扫描）。中英双语文案都改。
4. **后台/杀进程行为保持**：`CREATE_NO_WINDOW` + `CREATE_NEW_PROCESS_GROUP` + `taskkill /T /F` 进程树杀灭逻辑不变；后台 dev server（`npm run dev` 等）经 PowerShell 启动后仍能被正确追踪与 kill。
5. **既有安全/规范化逻辑不回归**：denylist、`allow_host_python_package_install` 守卫、`normalize_run_command`（`cd ... &&` 拒绝/剥离）、大输出 offload、null stdin 等全部保持。

## 非目标

- 不做用户可配置 shell（settings 里选 shell）——本次固定平台策略，后续可选。
- 不改 macOS/Linux 的 `sh -c` 执行。
- 不为 PowerShell 新增 Windows 专属 denylist 条目（`Remove-Item -Recurse` 之类）——记为后续；现有 denylist 保持。
- 不改 run_python 沙盒、不改 MCP/skill 执行路径。

## 验收标准

- [ ] Windows 上 `run_command` 执行的是 PowerShell：仅 PowerShell 可运行的构造（如 `Write-Output (1+1)` → `2`）能成功，而不是被 cmd 解析失败。
- [ ] 内部引号回归：`python -c "print(40 + 2)"` 仍输出 `42`（`windows_run_command_preserves_embedded_quotes` 通过）。
- [ ] 中文输出不乱码：`Write-Output '你好'` 的 stdout 解码后包含 `你好`。
- [ ] 后台命令仍被追踪、`bash_output` 能读到输出、`kill_background` 能真正杀掉进程组（后台相关测试通过）。
- [ ] 系统提示词 Windows 分支显示 `PowerShell` 且给出 PowerShell 语法引导（不再出现 `%VAR%`/`dir` 误导），中英双语一致。
- [ ] macOS/Linux 路径与测试不受影响（`sh -c` 分支不变）。
- [ ] `cargo` 相关测试通过（Windows 经 `scripts/win-cargo-test.ps1`，对照既有 ~14 项 env/locale --lib 基线失败，不引入新失败）。

## 备注

- 本机实测：只有 `powershell.exe`（5.1），无 `pwsh` → UTF-8 编码修复为必需项，须专门验证。
- 5.1 不支持 `&&` 串联；`normalize_run_command` 已在执行前剥离前导 `cd x &&`，提示词引导用 `;`，故不依赖 `&&`。pwsh 7 支持 `&&`，回退策略下更宽松。
- 关联记忆：[[windows-rust-test-manifest]]、[[windows-cargo-lib-preexisting-failures]]、[[streaming-toolcall-id-no-empty-overwrite]]（同为"抓 opencode 真实流量对比"定位法）。
