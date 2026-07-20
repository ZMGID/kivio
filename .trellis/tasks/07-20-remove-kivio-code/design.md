# Design — 删除 Kivio Code

## 关键决策:先搬迁 `app_data_dir`,再删目录

`kivio_code/settings_loader.rs` 里有多个函数,但**只有 `app_data_dir()`(+ 常量 `APP_IDENTIFIER`/`SETTINGS_STORE_FILE`)被 kivio_code 之外引用**。其余(`load_settings_from_disk`/`load_settings_from_path`/`settings_store_path`)只有 CLI 用,随目录一起删。

### 新模块 `src-tauri/src/app_data.rs`

```rust
//! 无 Tauri handle 的 per-app data 目录解析(供无 AppHandle 的模块使用)。

use std::path::PathBuf;

/// Tauri bundle identifier(须与 tauri.conf.json 一致)。
pub const APP_IDENTIFIER: &str = "com.zmair.kivio";

/// 与 Tauri `app_data_dir` = `dirs::data_dir()/<identifier>` 完全一致。
/// 注意:必须用 `BaseDirs::data_dir()`,不能用 `ProjectDirs`(Windows 会多一层 `\data`)。
pub fn app_data_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|base| base.data_dir().join(APP_IDENTIFIER))
}
```

在 `lib.rs` 加 `pub mod app_data;`。

### 3 处调用点重指

| 文件 | 原 | 新 |
|---|---|---|
| `connectors/himalaya.rs:31` | `crate::kivio_code::settings_loader::app_data_dir()` | `crate::app_data::app_data_dir()` |
| `plugins/state.rs:36` | 同上 | `crate::app_data::app_data_dir()` |
| `skills/discover.rs:75` | 同上 | `crate::app_data::app_data_dir()` |

`skills/discover.rs` 里对 `kivio_code::settings_loader::app_data_dir` 的文档注释(第 70 行)同步改成指向 `app_data`。

## 删除清单

### Rust
1. **整目录** `src-tauri/src/kivio_code/`(cli/config/errors/executor/host/mcp_setup/mod/project_context/settings_loader/skill_setup/vision + `interactive/` `tui/` `session/`)
2. `src-tauri/src/bin/kivio-code.rs`
3. `src-tauri/src/cli_install.rs`
4. `Cargo.toml`:删 `[[bin]] name = "kivio-code"` 块(3 行 + 注释);`default-run = "kivio"` 保留(现在只剩一个 bin,保留也无害;顺带清理注释)
5. `main.rs`:删 `if first == "code" { … }` 分支(第 18–25 行)。`rapidocr` worker 分支保留。
6. `lib.rs`:
   - 删 `pub mod kivio_code;`、`pub mod cli_install;`
   - 加 `pub mod app_data;`
   - 删 `invoke_handler` 里 5 个命令:`get_kivio_code_config`、`set_kivio_code_config`、`get_kivio_code_global_instructions`、`set_kivio_code_global_instructions`、`install_cli_command`
7. `commands.rs`:删这 5 个命令的实现函数 + `kivio_code_global_instructions_path` 辅助函数

### 前端
8. 删 `src/settings/KivioCodeSettings.tsx`
9. `src/settings/SettingsShell.tsx`:
   - 删 `import { KivioCodeSettings }`
   - `SettingsTab` union 去掉 `'kivioCode'`
   - 删导航项 `{ id: 'kivioCode', label: 'Kivio Code', … }`(~1893)
   - 删 tab 说明 map 里的 `kivioCode: {…}`(~1935)
   - 删渲染块 `{activeTab === 'kivioCode' && …}`(~3051)
10. `src/api/tauri.ts`:删 `KivioCodeConfig` 类型、`InstallCliResult` 类型、`getKivioCodeConfig`/`saveKivioCodeConfig`/`getKivioCodeGlobalInstructions`/`saveKivioCodeGlobalInstructions`/`installCliCommand` 5 个方法

## 兼容性 / 回滚

- 纯删除 + 一处小搬迁,无数据迁移。
- 用户曾建的 `<app_data>/kivio-code/`、`~/.local/bin/kivio` 软链、Windows PATH 项**不主动清理**(残留无害,越界清理反而有风险)。
- 回滚 = `git revert` 整个 commit。

## 风险

- **唯一编译风险**:遗漏某处 `kivio_code::` 引用。用 `grep -rn "kivio_code" src-tauri/src` 收尾核验(应为 0)。
- external_agents/attachments.rs、web_search.rs 里的 `kivio_code` 只是注释,改不改无碍功能;顺手清理注释即可。
