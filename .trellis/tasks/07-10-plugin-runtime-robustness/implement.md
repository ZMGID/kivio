# Implement: 插件即插即用 — Kivio 运行时健壮性

按依赖排序;每步一个 commit,独立可回滚。每步完成即运行该步的验证命令。

## 步骤

### 1. R4 — Windows run_command 优先 Git Bash
- [x] `native_tools/shell.rs`:`find_git_bash()`(OnceLock 缓存;已知安装位 ProgramFiles/x86/LocalAppData existsSync → `where.exe bash.exe` 第一行+存在复核;**排除 System32|sysnative**(WSL))
- [x] Windows 执行分支:bash 命中 → `bash.exe -c <cmd>`(`.arg()` 单参);否则现有 pwsh→powershell 原样
- [x] 背景任务(`background:true`)同样切换;确认 `kill_process_group` 不受影响
- [x] **run_command description 动态化**(`mcp/types.rs` / native_registry 构建处):bash → "bash 语法+Windows 路径用正斜杠 C:/…";PowerShell 回落 → 现有 PS 描述(opencode #16479/#15810 教训)
- [x] 单测:检测函数排除 System32;where 幽灵路径复核;bash 缺失回落
- 验证:`powershell -File scripts/win-cargo-test.ps1`(基线对照);GUI 手测 `cat <<EOF`、`for i in $(seq 1 3)`、管道、正斜杠路径 `ls C:/Users`

### 2. R2 — 辅助视觉模型审查向
- [x] `chat/commands.rs`:分析 prompt 追加审查指令(截断/溢出/重叠/字面转义/错位/对比度;逐条列出;确无才写"未见视觉缺陷");中英双语路径都改
- 验证:构造带字面 `\n` 的 PNG,`read` 之,分析文本显式指出缺陷(AC2)

### 3. R1 — MCP 工具结果图片直达模型
- [x] `chat/commands.rs`(或就近模块):`data_url_image_part(data_url) -> Value`(与 `image_content_part` 并列)
- [x] `mcp/registry.rs` MCP 执行点:`attach_image_artifacts_for_model(state, settings, conversation_id, &mut result)`
  - 收集 image artifacts(mime `image/*` 且 data_url 非空)
  - `model_supports_vision` == true → follow_up_user_messages 一条 user 消息带 ≤4 图;单图 ≤8MB,超限占位符注明
  - 非 vision → data_url 落临时 `kivio-mcpimg-*.png` → `auxiliary_vision_model_for_images` 审查向分析;失败保留占位符
  - `screenshot.rs::cleanup_orphan_temp_files` 加 `kivio-mcpimg-` 前缀 GC
- [x] 单测:artifact 过滤、护栏(超大/超数)、无图直通
- 验证:vision 模型对话跑 officecli screenshot → conv JSON model_messages 出现图块(AC1);非 vision → 审查向文字

### 4. R3 — 通用行为规范进 prepare.rs
- [x] `chat/agent/prepare.rs` 通用段(chat_tools.enabled 时)+三条:中间产物进临时目录 / 结束前清理 / stdio MCP 绝对路径
- [x] 若有系统提示相关快照测试,更新
- 验证:新对话系统提示含三条(request_debug 查看);`cargo test` prepare 相关

### 5. R5 — OfficeCLI hint 瘦身
- [x] `plugins/catalog.rs` system_hint 重写 ≤15 行:角色 + skill 映射 + 三 Do NOT(bash 跑 officecli / watch / mcp <ide>)+ Done
- [x] 删除:入口教学、batch 教学、截图落盘+read、绝对路径、效率细则
- 验证:`cargo check`;hint 行数

### 6. E2E 全链路验收
- [x] GUI 新对话:5 页 PPT(含对比页),观察:officecli 全 MCP、无 bash officecli、无中间文件卡片、交付目录只剩 .pptx、模型真实看图(发现植入缺陷并修复)
- [x] 扒 conv JSON:tool_calls 统计 + model_messages 图块(AC1–AC5 逐条)
- [x] `npm run lint && npm run typecheck && npm test`
- [x] `powershell -File scripts/win-cargo-test.ps1` 对照基线

## 回滚点

- 每步独立 commit;R4 删检测函数即回 PowerShell;R1 删 registry 调用点即回占位符行为;R5 是纯文本可随时还原。

## 风险与缓解

- R4 行为变化(存量 PowerShell 命令在 bash 失败):description 已更新,模型见错自改;严重则加设置开关(暂不做,ponytail)。
- R1 token 成本:护栏(≤4 图/结果、仅当轮);实测观察 usage 面板。
- R1 各协议适配器兼容:follow_up 机制 read 已验证;E2E 阶段至少验 OpenAI-compat + Anthropic 两协议。


## ?????2026-07-11?

- Windows shell ?????`scripts/win-cargo-test.ps1 --lib native_tools::shell::tests` ? **25 passed, 0 failed**??? Git Bash heredoc / `seq` / ???WSL Bash ??? PowerShell ???
- MCP ????????????`mcp_image_feedback` **7/7**?`vision` **3/3**????????? **1/1**?`agent::prepare` **19/19**?
- ??????`npm run lint`?`npm run typecheck`?`npm test -- --run` ?????Vitest **37 files / 201 tests passed**?
- Rust ???**1334 passed, 8 failed, 8 ignored**?8 ??????????????agent compaction 5 ??sandbox export 1 ??Unix/macOS PATH 2 ???????????????
- ?? OfficeCLI E2E?probe `plugin-runtime-20260711-082524`???????? `deepseek-v4-flash` ??????? `models/gemini-3.1-flash-lite` ?? MCP screenshot ?????????????? `\n`?????/????????????????????????????
- E2E ??????? OfficeCLI ????? `mcp__plugin-officecli__officecli`???? shell ???? `officecli.exe`??? `bash` ??????????????
- E2E ??????????? `kivio-plugin-runtime-e2e.pptx`??? JSON / PNG / ?? PPTX ????
- ?????`conv_c82645ff-9092-4c9e-a603-de22507d418d.json` ? `model_messages` ???????? screenshot ????????????????
