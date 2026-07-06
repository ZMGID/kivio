# Design — Windows 读取 PowerShell profile PATH

## 方案概述

在 `src-tauri/src/path_env.rs` 的 Windows 分支新增一个 PATH 来源:**以加载 profile 的方式运行用户 PowerShell,读出真实 `$env:PATH`**,与现有「进程 PATH + 注册表(system/user)+ 常见目录」合并。结构上完全对称 macOS 的 `login_shell_path()`。

选定方案 = PRD 里的「A 为主、B 兜底」:
- A:profile probe(通用,覆盖 fnm/nvm/scoop shim 等一切 profile 驱动的 PATH 注入)。
- B:`common_dirs_windows()` 增补 fnm/nvm 稳定目录(probe 失败或超时时的兜底)。

## 关键决策

### D1: 同步执行 + 硬超时(不做后台异步)

`enrich_path_*` 必须在任何线程/子进程 spawn 之前完成(`std::env::set_var` 在多线程下不安全;现有两个平台入口都在 `lib.rs::run()` 最前同步执行)。macOS 已接受「最多阻塞 3s」的先例。Windows 沿用同一模式:helper 线程 + `mpsc::recv_timeout`,超时即放弃。

超时值:**3s**(`PROFILE_SHELL_TIMEOUT`,与 mac `LOGIN_SHELL_TIMEOUT` 一致)。PowerShell 5.1 冷启动 + fnm profile 实测一般 <1.5s;pwsh 更快。超时打不满就是无感,打满说明用户 profile 本身慢,降级为现状行为。

### D2: PowerShell 选择与调用形态

- 复用 `shell.rs` 的选择逻辑:PATH 上有 `pwsh.exe` → `pwsh`,否则 `powershell`。**注意**:此判定必须发生在注册表合并**之后**(pwsh 常由 winget/msi 装在 `%ProgramFiles%\PowerShell\7`,进程的 stale PATH 可能没有它)——所以执行顺序是:先做现有注册表合并并 `set_var`,再跑 profile probe,再做第二次合并。`shell.rs::windows_powershell_exe` 是 `OnceLock` 且在 native tool 首次调用才初始化,启动早期不会互相干扰;probe 内部自己判定(不复用那个 OnceLock,避免过早固化)。
- 调用:`<exe> -NoLogo -NonInteractive -Command "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; $env:PATH"`。
  - **不加 `-NoProfile`** —— 加载 profile 正是目的。
  - UTF-8 前缀防非 ASCII 目录名 mojibake(PS 5.1 默认 OEM code page;pwsh 无害)。
  - `Stdio::null()` stdin,`piped` stdout,`null` stderr(profile 的横幅/警告不污染结果)。
  - `NoConsoleWindow`(`proc.rs`)防控制台闪现。
- 取输出**最后一个非空行** trim 后作为 PATH(profile 可能往 stdout 打印东西;`$env:PATH` 是最后一条输出)。含 `;` 校验:结果必须包含至少一个 `;` 或形如 `X:\` 的片段,否则视为失败丢弃(防 profile 打印文本被误当 PATH)。

### D3: 合并顺序与纯函数扩展

`merge_paths_windows(current, system, user, profile, defaults)` 增加 `profile: Option<&str>` 参数,来源顺序:

```
current(进程)→ system(注册表)→ user(注册表)→ profile(新)→ defaults
```

保持「现有解析顺序优先」不变(与 mac 一致);去重仍大小写折叠、首见拼写保留。已知取舍:若用户同时有系统安装的 node(registry PATH)和 fnm node,系统的赢——与 macOS 分支同等行为,文档注明,不在本任务内解决。

实现形态:`enrich_path_windows` 内部两段式——
1. 现有逻辑先合并 registry+defaults 并 `set_var`(保证 probe 能找到 pwsh);
2. probe 成功则以更新后的 PATH 为 `current` 再合并一次 `set_var`。失败/超时则第 2 步跳过,行为与今日完全一致(R6)。

### D4: 共享超时 helper

macOS `login_shell_path` 的「spawn + helper 线程 + recv_timeout」模式提取为共享私有 helper:

```rust
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn capture_stdout_with_timeout(cmd: std::process::Command, timeout: Duration) -> Option<String>
```

mac 分支重构为调用它(行为不变),Windows probe 复用。孤儿线程语义保持现状(detach,注释说明)。

### D5: 兜底稳定目录(方案 B)

`common_dirs_windows()` 增补(仅当对应基目录存在才 push,沿用现有 `push` 闭包风格):
- nvm-windows:`%NVM_SYMLINK%`(env 存在时);无 env 时不猜 `Program Files\nodejs`(那是系统 node 安装目录,盲加有误导)。
- fnm:`%FNM_DIR%` env 存在时 push `%FNM_DIR%\aliases\default`;否则探测默认根 `%LOCALAPPDATA%\fnm` 与 `%USERPROFILE%\.fnm` 下的 `aliases\default`。**注意**:fnm Windows 版 alias 目录内部结构(node.exe 直接在内还是 `installation\` 子目录)需在实现时实际验证,按验证结果决定 push 的最终路径;若结构不确定则两个候选都 push(不存在的目录在 PATH 里无害)。

### D6: 不改的东西

- `native_tools/shell.rs` 的 `-NoProfile`:工具执行 shell 保持快速确定性;node 可见性由进程 PATH 继承解决。
- macOS 分支行为(仅内部重构到共享 helper,输出不变)。
- 不新增设置项、不写注册表、不写 profile。

## 数据流

```
lib.rs::run()
  └─ enrich_path_windows()
       ├─ ① registry(system+user, expand %VAR%) + common_dirs → merge → set_var    [现状]
       ├─ ② profile_shell_path()                                                  [新]
       │     pwsh|powershell -NoLogo -NonInteractive -Command "...; $env:PATH"
       │     (3s 超时 / NoConsoleWindow / 最后非空行 / 合法性校验)
       └─ ③ probe 成功 → merge(current=①结果, profile=②) → set_var                 [新]
→ 之后所有子进程(run_command / MCP stdio / external agent 探测)继承
```

## 测试策略

- 纯函数单测(全平台编译,沿用现有 `#[cfg(test)]` 模式):
  - `merge_paths_windows` 带 profile 来源:顺序、去重(大小写折叠)、profile 为 None 时与旧行为一致。
  - probe 输出解析(最后非空行提取 + 合法性校验)提为纯函数 `parse_profile_path_output(&str) -> Option<String>` 单测。
- Windows 手工验收:fnm 环境启动 GUI,经 chat-probe 通道跑 `node -v`;拔掉 profile / 改坏 profile 验证降级;观察无控制台闪现。
- 单测经 `scripts/win-cargo-test.ps1` 运行(直接 cargo test 二进制会 0xC0000139)。

## 回滚

单模块改动(`path_env.rs` + `lib.rs` 注释),revert 一个 commit 即回到纯注册表行为;无数据/设置迁移。
