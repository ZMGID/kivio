# 原生工具集精简（MVP）

## Goal

Kivio 的基础原生工具已达 24 个，存在冗余，增加模型的工具选择认知负荷。参照 opencode（仅 11 个）与 Claude Code 的极简工具集，本 MVP 做 **4 项零/低回退风险的合并与改名**，把基础工具从 24 降到 20，同时**不丢任何能力、不破坏已有 persona/会话的工具白名单**。

skill 三件套合并（有安全约束回退风险）与 memory 三合一、bash 后台深度重构**明确不在本次范围**，留作二期单独设计。

## 决策（已与用户确认，2026-07-03）

- **skill 三件套（activate/read_file/run_script）本次不动**——`run_script` 的脚本 `scripts/` 前缀限制+解释器白名单+skill cwd+超时、`read_file` 的 skill 根相对路径+`secrets/` 访问，合并会丢失，风险不值当，延后。
- **`list_background` 删除，能力折进 `bash_output`**：不传 `job_id` 时返回后台作业列表（等价替代，不丢功能）。
- 改名/移除的旧工具名一律保留**兼容别名**，保证向后兼容。

## Scope（本次 4 项）

### R1：`read` 读目录 + 移除 `ls`
- `read` 传入目录路径时，返回该目录的条目列表（复用 `list_dir` 现有 JSON 形态）。
- 移除独立 `ls` 工具；旧名 `ls` 归一化到 `read`（模型调用 + 存储白名单两处）。
- 目录场景下 `offset/limit` 的行为在 design 里定；不破坏文件读的既有行为。

### R2：移除 `list_background`，能力并入 `bash_output`
- `bash_output` 不传 `job_id` 时，返回当前 app 会话所有后台作业（job_id/status/command/cwd/age）——等价 `list_background`。
- 传 `job_id` 时行为不变（增量读该作业输出）。
- 移除独立 `list_background` 工具。

### R3：`todo_update` 并入 `todo_write`，移除 `todo_update`
- `todo_write`（整表替换）已能覆盖单项增删改。移除 `todo_update`。
- 确认 `todo_update` 没有 `todo_write` 无法表达的能力（design 阶段核对 `apply_todo_update` vs `apply_todo_write`）。

### R4：`find` 改名 `glob`
- 工具的真实 `name` 改为 `glob`（对齐 opencode/Claude Code，提升模型识别率），行为不变。
- 旧名 `find` 加兼容别名（模型调用 + 存储白名单两处解析）。

## 向后兼容（贯穿 R1/R3/R4）

- 内置 persona（`agents/types.rs`）、用户 `<app_data>/agents/*.md`、项目 `.kivio/agents/*.md` 里写的 `ls`/`find`/`todo_update` 白名单条目，经别名/归一化后仍能匹配到新工具，**不被静默剔除**。
- 若存在会话持久化的工具名列表（design 阶段核实 `chat/storage.rs`），同样经归一化处理。

## Acceptance Criteria

- [ ] AC1：基础工具数 24 → 20（移除 `ls`/`list_background`/`todo_update`，`find`→`glob` 改名）。
- [ ] AC2：`read` 传目录路径返回条目列表；传文件仍按行号读，既有行为不回退。
- [ ] AC3：`bash_output` 不传 `job_id` 返回作业列表；传 `job_id` 增量读不变。
- [ ] AC4：`todo_write` 能完成原 `todo_update` 的单项增/删/改场景（至少一条覆盖）。
- [ ] AC5：模型发出旧名 `ls`/`find`/`todo_update` 的工具调用，仍能正确路由到新工具（别名生效）。
- [ ] AC6：persona 白名单里写 `ls`/`find`/`todo_update` 的 sub-agent 不因改名而丢工具。
- [ ] AC7：`cargo check --lib --tests` 干净；受影响的注册表快照测试 / `loop_tests` 更新并通过（cargo test 环境问题时 harness 验证）；`npm run typecheck/lint/test` 全绿。
- [ ] AC8：涉及工具名的系统提示词/spec/CLAUDE.md 引用同步更新。

## Notes

- 范围外：skill 三件套合并、memory 三合一、bash 后台流式重构、`ls` 的元数据（大小/隐藏）若在目录读里精简需注明。
- 前端 `ChatNativeToolsConfig` 开关**不需改**——readFile 已同时管 read/ls/grep/find，runCommand 管 bash 全家，合并的都是同组工具。
- 研究详情见 `research/tool-inventory.md`、`research/consolidation-touchpoints.md`、`research/risks-and-open-questions.md`。
- 别名机制注意：现有 `web_search`→`search_web` 是 **wire 别名**（改模型可见名、内部名不变），与本次的 `find`→`glob`（改内部真实名 + 旧名兼容解析）方向相反，不能直接照搬，design 需区分。
