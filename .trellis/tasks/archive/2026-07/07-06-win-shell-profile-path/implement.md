# Implement — Windows 读取 PowerShell profile PATH

前置:阅读顺序 implement.jsonl → prd.md → design.md → 本文件。改动集中在 `src-tauri/src/path_env.rs`,入口注释在 `src-tauri/src/lib.rs`。

## Checklist

### 1. 共享超时 helper(重构,行为不变)
- [ ] 在 `path_env.rs` 新增 `capture_stdout_with_timeout(cmd, timeout) -> Option<String>`(`#[cfg(any(target_os="macos", target_os="windows"))]`):spawn(调用方已设好 stdio/NoConsoleWindow)→ helper 线程 `wait_with_output` → `recv_timeout`;成功且 exit ok → `Some(stdout_lossy)`;其余 `None`,孤儿线程 detach(保留现有注释语义)。
- [ ] macOS `login_shell_path()` 改为构造 Command 后调用该 helper,输出处理(trim/empty→None)不变。

### 2. Windows profile probe(新)
- [ ] `PROFILE_SHELL_TIMEOUT: Duration = 3s` 常量(cfg windows)。
- [ ] `profile_shell_exe() -> &'static str`:PATH 扫描 `pwsh.exe`(参考 `shell.rs::pwsh_on_path`,本地实现,不复用其 OnceLock),有则 `"pwsh"` 否则 `"powershell"`。
- [ ] `profile_shell_path() -> Option<String>`(cfg windows):构造 `<exe> -NoLogo -NonInteractive -Command "try{[Console]::OutputEncoding=[System.Text.Encoding]::UTF8}catch{}; $env:PATH"`,stdin null / stdout piped / stderr null / `no_console_window()`,经 helper 执行,输出交给解析函数。
- [ ] `parse_profile_path_output(&str) -> Option<String>`(`#[cfg(any(target_os="windows", test))]` 纯函数):取最后一个非空行 trim;合法性校验(含 `;`,或匹配 `^[A-Za-z]:\\`);不合法 → None。

### 3. 合并逻辑扩展
- [ ] `merge_paths_windows` 增加 `profile: Option<&str>` 参数,来源顺序 current → system → user → profile → defaults;更新现有调用点与测试。
- [ ] `enrich_path_windows` 改两段式(design D3):①现有 registry+defaults 合并 + `set_var`;② `profile_shell_path()`;③ Some 时以①后的 PATH 为 current 再合并 + `set_var`,None 时跳过。

### 4. 兜底稳定目录
- [ ] `common_dirs_windows()` 增补:`%NVM_SYMLINK%`(env 存在时);fnm alias default 目录(`%FNM_DIR%` → 否则 `%LOCALAPPDATA%\fnm` / `%USERPROFILE%\.fnm`,push `aliases\default`;实现时实际验证 fnm Windows 目录结构,必要时同时 push `aliases\default\installation`)。

### 5. 文档与注释
- [ ] `path_env.rs` 模块 doc:Windows 段补充 profile probe 机制说明(为什么注册表覆盖不到 fnm)。
- [ ] `lib.rs` 调用处注释同步一句。

### 6. 测试
- [ ] 新增单测:`parse_profile_path_output`(正常 PATH / profile 打印噪音后跟 PATH / 空输出 / 纯文本无 `;` 拒收 / 单盘符路径接受)。
- [ ] 更新/新增 `merge_paths_windows` 测试:profile 来源在 user 之后 defaults 之前;`None` 时结果与旧签名行为一致;大小写去重跨 profile 来源。
- [ ] 全量:`powershell -File scripts/win-cargo-test.ps1`(记忆:直接跑 cargo test 二进制 0xC0000139;对照已知 ~14 个环境性 --lib 基线失败,不算回归)。

## 验证命令

```powershell
powershell -ExecutionPolicy Bypass -File scripts/win-cargo-test.ps1
```

手工验收(有 fnm 的 Windows 环境):
1. PowerShell profile 含 `fnm env --use-on-cd | Out-String | Invoke-Expression`,终端 `node -v` 正常。
2. `npm run dev` 或打包版启动 → chat 里 `run_command: node -v` → 返回版本号。
3. 临时改坏 profile(抛异常)再启动 → 功能与当前版本一致,启动无卡顿超 3s、无控制台闪现。

## Review gates

- Gate A(实现后):trellis-check 过 path_env 全部单测 + lint;确认 macOS 分支仅重构无行为 diff。
- Gate B(合并前):手工验收 1–3 完成;不满足则回 Phase 2。

## 回滚点

单 commit 改动 `path_env.rs`(+`lib.rs` 注释),`git revert` 即恢复纯注册表行为。
