# Implement — 删除 Kivio Code

按顺序执行。**先搬迁再删除**,避免中间态编译失败。

## 步骤

### 1. 搬迁 `app_data_dir`
- [ ] 新建 `src-tauri/src/app_data.rs`(见 design.md 内容)
- [ ] `lib.rs` 加 `pub mod app_data;`
- [ ] `connectors/himalaya.rs:31` 改 `crate::app_data::app_data_dir()`
- [ ] `plugins/state.rs:36` 改 `crate::app_data::app_data_dir()`
- [ ] `skills/discover.rs:75`(+ 注释 70 行)改 `crate::app_data::app_data_dir()`

### 2. 删后端命令注册与实现
- [ ] `lib.rs`:删 `pub mod kivio_code;` + `pub mod cli_install;`;删 invoke_handler 里 5 个命令
- [ ] `commands.rs`:删 5 个命令实现 + `kivio_code_global_instructions_path`

### 3. 删入口
- [ ] `main.rs`:删 `code` 子命令分支
- [ ] `Cargo.toml`:删 `[[bin]] kivio-code` + 相关注释

### 4. 删文件/目录
- [ ] `rm -rf src-tauri/src/kivio_code/`
- [ ] `rm src-tauri/src/bin/kivio-code.rs`(空则删 `src/bin/` 目录)
- [ ] `rm src-tauri/src/cli_install.rs`

### 5. 前端
- [ ] `rm src/settings/KivioCodeSettings.tsx`
- [ ] `SettingsShell.tsx`:import / union / 导航项 / 说明 map / 渲染块
- [ ] `tauri.ts`:2 个类型 + 5 个 api 方法

### 6. 收尾注释(可选,低风险)
- [ ] `external_agents/attachments.rs:22`、`web_search.rs:651` 注释里的 kivio_code 提法可留可清

## 验证命令

```bash
# 后端编译(默认 kivio bin)
cargo build --manifest-path src-tauri/Cargo.toml

# 前端
npm run typecheck
npm run lint

# 残留核验(应无功能性命中)
grep -rn "kivio_code\|cli_install\|KivioCode\|kivioCode" src src-tauri/src
grep -rn "install_cli_command\|installCliCommand\|kivio-code" src src-tauri/src
```

## 回滚点

每个大步(1/2-3/4/5)后都可编译。若步骤 4 后编译失败 → 必是步骤 1 漏搬或步骤 2-3 漏删引用,按 grep 结果补。整体回滚 = `git checkout .` + `git clean -fd`(尚未 commit 时)。

## 检查门

- 步骤 1 完成后先 `cargo check` 一次,确认搬迁无误(此时 kivio_code 仍在,应编译通过)。
- 全部完成后跑完整验证命令 + 手动启动 `npm run dev` 冒烟设置面板(确认无 Kivio Code tab、其余 tab 正常)。
