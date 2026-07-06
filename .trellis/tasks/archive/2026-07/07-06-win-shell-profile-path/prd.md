# Windows 读取 PowerShell profile PATH:支持 fnm/nvm 安装的 node

## Goal

Windows 上通过 fnm / nvm(及其他依赖 PowerShell profile 动态注入 PATH 的版本管理器)安装的 node 等 CLI,Kivio 的 agent 工具(`run_command`、外部 CLI agent 探测、MCP stdio 服务器等)当前找不到,用户被迫用 vbs 环境注入等 workaround。目标:启动时的 Windows PATH 修复(`path_env.rs`)在现有「注册表读取」之外,补充「加载用户 PowerShell profile 读取真实 `$env:PATH`」,使 fnm/nvm 场景开箱即用。

## Background / 根因

- `path_env::enrich_path_windows` 只读注册表(HKCU/HKLM `Path`)+ 硬编码常见目录;**从不执行 PowerShell profile**。
- fnm 的机制是在 profile 里 `fnm env | Invoke-Expression`,把 per-shell 动态目录(`%LOCALAPPDATA%\fnm_multishells\<pid>_<ts>`)前插到 PATH——该目录不在注册表、无固定值,注册表方案原理上覆盖不到。
- macOS 分支(`login_shell_path`)已经是「跑登录 shell 读 PATH」模式,Windows 缺对称实现。
- `native_tools/shell.rs` 的 `run_command` 刻意 `-NoProfile`(启动快、行为确定),不改;修复点在进程级 PATH,子进程继承即可。

## Requirements

- R1: Windows 启动 PATH 修复新增来源:以加载 profile 的方式运行用户 PowerShell,读取其 `$env:PATH`,合并进进程 PATH(去重、保序,进程 PATH 优先)。
- R2: 硬超时(与 macOS `LOGIN_SHELL_TIMEOUT` 对称,约 3s),超时/失败静默回退到现有注册表+默认目录逻辑;绝不阻塞、绝不 panic、不弹控制台窗口(用 `proc.rs::NoConsoleWindow`)。
- R3: PowerShell 选择与 `shell.rs` 一致:优先 `pwsh`,回退 `powershell`(两者 profile 路径不同,选错会读不到用户实际配置;fnm 官方文档以用户日常 shell 为准)。
- R4: 兜底稳定目录:`common_dirs_windows()` 增加 fnm/nvm 的稳定路径(nvm-windows 的 `NVM_SYMLINK`/默认 `C:\Program Files\nodejs`;fnm 的 `FNM_DIR\aliases\default` 类稳定别名目录,如存在)。
- R5: 读到的 PATH 只做合并增强,不覆盖/删除现有条目;仍为只读操作(不写注册表、不写 profile)。
- R6: 失败模式全部降级:无 profile、profile 报错、PowerShell 不存在、输出为空 → 与今日行为完全一致。

## Non-goals

- 不改 `run_command` 的 `-NoProfile` 策略(工具 shell 保持快速、确定性)。
- 不做 PATH 的运行期动态刷新(仍是启动一次性修复,与现有架构一致)。
- 不处理 macOS/Linux(macOS 已有对称实现)。
- 不引入用户可见设置项(默认开启,行为对用户透明)。

## Risks / Constraints

- **执行用户 profile 有副作用风险**(profile 是任意代码):仅在启动时执行一次,与 macOS 跑 `.zshrc` 的既有先例一致;超时兜底。
- **fnm multishell 目录生命周期**:probe shell 退出后 `fnm_multishells\<id>` 目录通常保留(fnm 仅 GC 陈旧目录),node.exe 经 symlink 仍可用;但需在验收中确认长时间运行后仍有效。R4 的稳定别名目录作为二道保险。
- **启动性能**:PowerShell 冷启动 + profile 加载可能 1–3s;必须在后台/带超时,不能拖慢窗口首绘(评估是否可与现有启动序列并行,见 design)。
- Windows PowerShell 5.1 输出编码(OEM code page)不影响纯 ASCII 的 PATH 读取,但含非 ASCII 目录名的 PATH 需正确解码(复用/参考 `wrap_ps_command` 的 UTF-8 前缀)。

## Acceptance Criteria

- [ ] Windows + fnm(PowerShell profile 配置 `fnm env | Invoke-Expression`)环境:启动 Kivio 后,chat 里 `run_command` 执行 `node -v` 成功返回版本号,无需任何 vbs/手动环境注入。
- [ ] Windows + nvm-windows 环境:同上,`node -v` 成功。
- [ ] 无 profile / profile 抛错 / 模拟 PowerShell 缺失时:启动不报错、不变慢超过超时上限,PATH 修复结果与当前版本一致(注册表+默认目录)。
- [ ] 启动无控制台窗口闪现。
- [ ] 纯函数合并逻辑有单元测试(profile PATH 参与合并的顺序、去重、大小写折叠);现有 `path_env` 测试全绿(经 `scripts/win-cargo-test.ps1` 运行)。
- [ ] macOS 行为零变化(改动 cfg 隔离在 Windows 分支)。
