# PRD — 后台进程 / dev-server 运行状态管理重构

## 执行记录（2026-07-03）
用户拍板：本次**只交付 D1（消弹窗）+ D4（环境净化）**；D2（yield window / 首返回带状态）、D3（就绪提示）**延后**（若日后仍遇"反复重启/看不懂状态"再做 D2）。已实现 + 单测通过，见 `src-tauri/src/native_tools/shell.rs`（`apply_shell_tool_env` + 后台 `creation_flags`）。D4 刻意不含 `CI=1`（避免改变工具行为）。


## 背景与问题
Kivio chat 的 agent 用 `run_command` 跑 dev server（如 `npm run dev`、electronmon）时，用户观察到两个问题：
1. **弹出黑色命令行窗口**（Windows）。
2. **反复重启 / agent 看不懂运行状态**：agent 启动了后台进程，但拿不到清晰的"已起来/健康"信号，于是反复重新启动，多个实例抢端口 → 更乱。

根因（见 `research/kivio-baseline.md`）：
- 后台 spawn 用了 `CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`，`DETACHED_PROCESS` 使 `CREATE_NO_WINDOW` 失效、子孙进程各自新建可见控制台。
- 后台状态只有裸 `running` + 原始日志流，无就绪语义、无初始输出（启动即空返回）；且无任何防重复启动机制，仅一句文字提示。

## 设计哲学（用户定调）
**靠模型 + 提升可见性，不靠控制/状态限制。** 模型反复拉扯是因为它"看不清当前状态"，而不是缺乏约束。所以本次只做"让模型看清运行状态"的改进，不做"拦着模型别怎样"的控制逻辑。

## 调研结论（见 `research/*.md`）
- Kivio 现有后台系统已 ≥ 业界（opencode V2/pi/aider 无后台；仅 Claude Code 同级）。→ **精修，不重造**。
- **yield window**（启动后等一个有界宽限期、命中退出或超时再返回初始输出+状态）被 codex（250ms–30s）与 gemini（200ms）双重验证 → 采纳。
- 就绪检测（up/URL/port）与防重复启动业界都没有；结合上面的哲学 → **防重复不做**，就绪只做轻量"可读性提示"（帮模型看，不改变行为）。
- 可借鉴：codex 环境净化（`TERM=dumb`/`NO_COLOR`/`PAGER=cat`/`CI=1`）。
- Kivio 的 `taskkill /T` + SIGTERM→SIGKILL grace 已最强 → 保留。

## 目标（本次范围）
1. **G1 消除弹窗**：Windows 后台命令不再弹出任何可见控制台窗口（含其 npm/vite/electron 子孙进程）。
2. **G2 状态可读（核心）**：`run_command(background)` 首次返回即包含"初始输出 + 明确状态（running/exited(code)）"，让模型一眼判断，而非空返回后盲 poll。
3. **G3 就绪可读性提示（轻量、可选）**：`bash_output` 对 running 的 dev server 附带就绪 marker 提示（只提示、不改状态、不限制）。
4. **G4 日志更干净**：对子进程做环境净化，减少交互/彩色/分页噪音。
5. **保持兼容**：`bash_output` 偏移轮询、`kill_background`、无 job_id 列表、日志文件格式、跨轮存活 —— 契约不变。

## 非目标（本次不做 / 明确排除）
- **防重复启动 / 幂等 / 任何控制类限制**：明确不做（用户定调：靠模型+可见性，不靠限制）。
- **Kivio 内置终端 / 实时交互 PTY**：明确不做（用户已否，现有 opencode/codex 体验够用，不必增此复杂度）。
- 完整就绪状态机 / 端口探测 / URL 结构化：不做（业界无先例、收益不明）。
- completion-injection（进程退出主动注回对话）：列后续可选增强，本次不做。
- ConPTY 化 / 前台 `run_command` 行为变更：不做。

## 验收标准
- **AC1**：Windows 上 `run_command` 跑 `npm run dev` / electronmon 类命令，无可见控制台窗口弹出（手动冒烟）。
- **AC2**：后台命令首次返回包含 job_id + 前 N 秒的初始输出 + 状态字段；若命令在宽限期内即退出（如启动失败），首次返回直接给出 exit_code + 输出（等同前台失败），不再让模型误以为"还在后台跑"。
- **AC3**：后台/前台子进程环境含 `TERM=dumb`、`NO_COLOR=1`、`CI=1`、`PAGER=cat`（单测覆盖 env 注入）。
- **AC4**：既有 shell 单测全绿（用 `scripts/win-cargo-test.ps1` 在 Windows 跑），偏移轮询 / kill / 列表契约不回归。

## 约束
- 改动集中在 `src-tauri/src/native_tools/shell.rs`（+ 可能 `state.rs` 的注册表查询）。
- Windows 单测须经 `scripts/win-cargo-test.ps1`（测试二进制缺 Common-Controls v6 清单会 0xC0000139）。
- 保持 `run_command` 工具 schema 向后兼容（不删参数）。
