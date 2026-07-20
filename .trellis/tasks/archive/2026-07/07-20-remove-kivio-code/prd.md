# PRD — 删除 Kivio Code 终端 agent 功能

## 背景

Kivio 目前是「一个 agent loop,多个宿主」的架构。其中 **Kivio Code**(`kivio_code/` 模块 + `kivio-code` binary + `kivio code` 子命令)是一个 headless/TUI 的终端编码 agent 宿主。现在决定整体删除这部分功能。

依赖方向是单向的:`kivio_code` 依赖 `chat::agent`,反之不成立。因此删除主要自洽,但存在一个共享工具函数需要先搬迁(见约束)。

## 目标

彻底移除 Kivio Code 相关的:
- Rust 模块 `src-tauri/src/kivio_code/`(含 `interactive/` `tui/` `session/` 子目录)
- `kivio-code` binary(`src/bin/kivio-code.rs` + `Cargo.toml` 的 `[[bin]]`)
- `kivio code` 子命令派发(`main.rs`)
- CLI 安装功能 `cli_install.rs`(仅为 `kivio code` 存在)+ `install_cli_command` 命令
- 相关 Tauri 命令(config / global instructions / install cli)
- 前端设置页 `KivioCodeSettings.tsx` + 导航 tab + `tauri.ts` API

## 约束 / 不能破坏的点

1. **`app_data_dir()` 必须保留**。`kivio_code::settings_loader::app_data_dir`(无 Tauri handle 的路径解析)被 3 个非 kivio_code 模块使用:
   - `connectors/himalaya.rs`
   - `plugins/state.rs`
   - `skills/discover.rs`(`user_skills_dir_headless`)
   直接删整个目录会编译失败。必须先把该函数搬到独立模块,再重指调用点。

2. **`_headless` 基础设施保留**。`AppState::new_headless`、`skills::build_registry_headless`、`user_skills_dir_headless` 等被 chat 模型测试、`request_debug`、`native_tools/shell`、`mcp/manager` 等广泛使用,**不是 kivio-code 独有**,不得删除。

3. **`directories` crate 保留**。除 kivio_code 外,`settings.rs`、`connectors/`、`external_agents/` 也在用。

4. 共享 agent loop(`chat::agent`)、external_agents、connectors 等其他宿主不受影响,行为不变。

## 验收标准

- [ ] `src-tauri/src/kivio_code/` 目录不存在
- [ ] `src-tauri/src/bin/kivio-code.rs`、`cli_install.rs` 不存在
- [ ] `Cargo.toml` 无 `kivio-code` bin 定义
- [ ] `main.rs` 无 `code` 子命令分支
- [ ] `cargo build --manifest-path src-tauri/Cargo.toml` 成功
- [ ] `npm run typecheck` + `npm run lint` 通过
- [ ] 设置面板无 "Kivio Code" tab;`grep -ri "kivio.code" src src-tauri/src` 仅剩注释/无关命中
- [ ] himalaya / plugins / skills 三处 `app_data_dir` 路径解析行为不变(改用新模块)

## 非目标

- 不清理 `clap`/`ratatui`/`crossterm` 依赖(可能变成未用;留作可选收尾,不阻塞验收)
- 不改 CLAUDE.md(文档更新在收尾阶段单独处理)
