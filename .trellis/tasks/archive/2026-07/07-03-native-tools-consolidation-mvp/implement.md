# Implement：原生工具集精简（MVP）

前置：读 prd.md + design.md + research/。工具数目标 24→20（移除 ls / list_background / todo_update，find→glob 改名）。

## 执行顺序

### Step 0：核实 + 归一化表（地基）
- [x] 核实 `chat/storage.rs` 无 per-conversation 工具名列表（确认）。
- [x] `mcp/types.rs`：新增 `LEGACY_TOOL_ALIASES` + `canonical_tool_name(name)`（注释标明与 wire alias 方向相反）。
- [x] `chat/agent/execute.rs::match_tool_call`：精确匹配失败后用 `canonical_tool_name` 再比一轮。
- [x] `chat/agent/prepare.rs::tool_matches_recommended_name`：比对前对 recommended `name` 过 `canonical_tool_name`。
- [x] 单测：`canonical_tool_name` 映射（types.rs）+ `match_tool_call` 旧名路由（execute.rs）。

### Step 1：R4 `find`→`glob`
- [x] `types.rs`/`native_registry.rs`/`prepare.rs`/`agents/types.rs` 全部改名；kivio_code 共享 def 连带更新（executor/app/tool_card/tests 接受 `glob`）。

### Step 2：R1 `read` 读目录 + 移除 `ls`
- [x] `call_read_file` 加 `is_dir()` 分支复用 `list_dir`；chat 移除 `ls` 注册条目；`read` 描述更新。
- [x] `native_list_dir_tool` def 保留（kivio_code 仍用 `ls`，隔离 MVP 范围到 chat）。

### Step 3：R2 移除 `list_background` → 并入 `bash_output`
- [x] `bash_output` `job_id` 改可选 + 无 job_id 走列表；删 `native_list_background_tool` def + `list_background`/`call_list_background` 注册；`shell.rs::list_background` fn 保留复用。

### Step 4：R3 `todo_update` 并入 `todo_write`
- [x] 删 `todo_update` 全部（def/args/apply/const/registry）；简化 `apply_tool`/`tool_definitions`/`is_agent_todo_tool_name`/`format_prompt`；改写 3 处单测为 `todo_write`。

### Step 5：快照测试 + 提示词 + spec/文档
- [x] 更新全部注册表快照（EXPECTED_ORDER/session_consent/parallel_safe/read_only/approval_bypass/builtin_exposure_snapshot）+ types.rs default_native_config/file_tool_path + files.rs 文件工具集 + commands.rs plan-filter 测试。
- [x] prepare.rs 提示词工具名核对（action_examples / 背景命令提示 / consent 注释）。
- [x] CLAUDE.md 后台轮询描述更新（list_background→bash_output 无 job_id）。spec 别名机制补充留 update-spec 步。

### Step 6：全量检查 + 手测
- [x] `cargo check --lib --tests` 干净（仅既有无关 warning）。
- [~] cargo test / harness：本环境 0xC0000139（测试 exe + example 均无法加载，DLL 导出不匹配）——逻辑靠编译通过 + 复核；行为靠运行中 app 手测。
- [x] `npm run typecheck && lint` 全绿；`test` 未受影响。
- [ ] 手测：app 内 chat 用 glob/read(目录)/bash_output(列表)/todo_write；旧名 find/ls/todo_update/list_background 仍路由；带 find/ls 白名单 sub-agent 不丢工具。（待用户在运行中的 app 验证）

## 回滚点
- Step 0 归一化表纯新增；Step 1–4 各自独立，可逐项 revert。出问题优先回退对应 Step。
- 每个 Step 一个 commit（Conventional Commits：`refactor(chat): ...` / `feat(chat): ...`）。

## Review gate
- Step 0 后自查两处消费点都接了归一化（漏一处则旧名在该侧失效）。
- Step 5 后确认无残留旧名引用（grep `todo_update`/`list_background`/`"ls"`/`"find"` 非注释非别名表处）。
- Step 6 结束跑 trellis-check 再进 Phase 3。
