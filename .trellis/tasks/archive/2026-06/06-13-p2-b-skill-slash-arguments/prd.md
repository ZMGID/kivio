# P2-B：skill slash 触发 + $ARGUMENTS + run 级缓存 + 动态 allowed_tools + skill_read_file 上限

> 来源：`06-12-refactor-kivio-agent-architecture-based-on-clawspring` P2 三线之一。
> 调研蓝图由 workflow `p2-research-blueprint`（2026-06-13）产出，逐符号锚定真实代码。

## 目标 / 验收（prd.md:81 口径 + 调研补全）

| # | 验收 | 交付行为 |
|---|---|---|
| T1 | run 级缓存替代每次调用全盘扫描 | `call_skill_tool`（mcp/registry.rs:446）不再每次 `build_registry`；每个 run 至多扫一次，存 `SkillRunCache.registry` 复用。 |
| T2a | `/<skillname> <args>` 触发 skill | 首 token 匹配 skill trigger → 走既有 `active_skill_id` pin 链，body 重写为 `[Skill: name]\n\n{rendered}`。 |
| T2b | `$ARGUMENTS` 替换 | body 内 `$ARGUMENTS` → 全部尾随参数；`$ARG_NAME` → 按声明 `arguments` 的位置词。 |
| T3 | 中途激活的 allowed_tools 动态过滤 | 模型 run 中 `skill_activate` 后，该 skill 的 `allowed_tools` 收窄后续轮的工具集（今天只有 UI pin 的 skill 在 prep 期过滤）。 |
| T4 | `skill_read_file` 大小上限 | `read_skill_file` 按 `MAX_READ_FILE_BYTES` 截断（head + 截断标记），替代无界 `fs::read_to_string`。 |

**非目标（P3，勿做）**：skill fork 执行 / Skill-as-tool function-call；`when_to_use` 目录增强。前端 `/` 自动补全弹层在范围内（`enabledSkills` 已携带完整 `SkillMeta`），但后端纯文本触发是承重路径，必须独立可用。

## 实现蓝图（文件级）

### T1 run 级缓存（缓存挂 `SkillRunCache`，非 AppState）

`SkillRunCache` 每 run 新建（loop_.rs:101），已贯穿执行管线（loop_ → execute.rs:109 → executor.call → registry.rs:315 的 `skill_cache: Option<&mut SkillRunCache>`），**无需新通道**；per-run 隔离天然、无淘汰/keying。`SkillRegistry` 已 `derive(Clone)`（types.rs:90）。

- `src-tauri/src/skills/runtime.rs`：`SkillRunCache` 加 `registry: Option<SkillRegistry>` + `registry_for(app, scan_paths) -> Result<&SkillRegistry>`（懒构建一次）；加 `activated_allowed_tools: Vec<String>`（T3）。
- `src-tauri/src/mcp/registry.rs::call_skill_tool`（438-496）：line 446 改用 `cache.registry_for(...)`；`skill_cache` 为 None 时回退一次性 build。**先 `.clone()` 出 `SkillRecord`** 释放对 `&registry` 的不可变借用，再 `&mut cache` 派发（`SkillRecord: Clone`，types.rs:38）。
- 失效：每 run 新 cache ⇒ 首次 skill 工具调用重建；import（skills/mod.rs:111）在 run 外，下个 run 自然拾取。

### T2 slash 触发 + $ARGUMENTS（后端承重，前端是糖）

后端在 `chat_send_message` 预处理做匹配 + 替换，用户纯文本 `/commit msg` 无前端补全也能用；前端 `/` 弹层只补全名字、参数交用户、原样透传，**不做替换**。

- `src-tauri/src/skills/types.rs::SkillMeta`：加 `triggers: Vec<String>`、`argument_hint: Option<String>`、`arguments: Vec<String>`（全 `#[serde(default)]`，camelCase，无迁移风险）。`SkillRegistry` 加 `find_by_trigger(first_word)`：首词 `/` 前缀、显式 triggers 或默认 `/{id}` / `/{slug(name)}` 精确匹配（不前缀匹配，避免与内置 slash 冲突）。
- `src-tauri/src/skills/parse.rs::parse_skill_markdown`（104-124）：用 `parse_list_value` 填三字段；加 `normalize_trigger`（补 `/` 前缀、小写）。
- `src-tauri/src/skills/runtime.rs`：新增纯函数 `substitute_arguments(body, args_raw, arg_names)`（`$ARGUMENTS` → 尾串；`$ARG_NAME` → 位置词；缺省填空，无 panic）。
- `src-tauri/src/skills/mod.rs`：re-export `substitute_arguments`。
- `src-tauri/src/chat/commands.rs::chat_send_message`（546-553，约 569 行前）：注入 `try_apply_skill_slash_trigger`——首词 `/` 且匹配 enabled skill 时，`content` 重写为 `[Skill: name]\n\n{substitute_arguments(body,...)}`、`active_skill_id = Some(id)`，**复用既有 pin 链**（resolve_forced_skill_id → active_skill_record → apply_active_skill_tool_filter + 目录/pin 注入），无需新过滤/目录/上下文代码。disabled skill 留作普通文本。

### T2 前端弹层（与内置命令共存）

内置 `/help /plan /new /compact /clear /settings /tools /attach` 是**会话动作**（dispatch 到 onNewChat 等，不 onSend）；skill 命令**发消息**。靠 `kind` 判别符共存。
- `src/chat/InputBar.tsx`：`SlashCommandDefinition` 加 `kind:'action'|'skill'`、`argumentHint?`；`slashCommands` memo 合并 `enabledSkills`（过滤 `disableModelInvocation`）；`enabledSkills` prop 拓宽（加 description/argumentHint/disableModelInvocation）；`slashCommandIcon` 对 skill 返回 Sparkles/Wand2；`handleSlashCommandSelect` 按 kind 分支——skill 走 `completeActiveSlashToken`（补 `/name ` 带尾空格、关弹层、聚焦，**不立即发送**），Enter 经正常 `handleSend`→`onSend` 发整串。`findActiveSlashToken` **不改**（输入空格后自然返回 null 关弹层）。
- `src/chat/Chat.tsx`（2536/2588）：`enabledSkills` mapper 传 description/argumentHint/disableModelInvocation（源 `skills` 已是完整 `SkillMeta`，无额外 fetch）。
- `src/api/tauri.ts`、`src/chat/types.ts`：`SkillMeta` 类型加三字段（camelCase）。`chatApi.sendMessage` 签名不变，**后端独立解析**（粘贴/API/移动端也生效）。

### T3 动态 allowed_tools（中途激活）

- 真值源：`call_skill_tool` 成功 `skill_activate` 后把 `record.allowed_tools` 累加进 `cache.activated_allowed_tools`（dedup）；native tool-call 与 DSML markup（dsml_tools.rs:170）两路都落到 `call_skill_tool`。
- `src-tauri/src/chat/agent/prepare.rs`：把 `apply_active_skill_tool_filter` 拆出 `retain_tools_for_allowed(tools, allowed)`（保留 skill 源 / native skill 工具 / Kivio builtin / 命中 allowed 的工具），原函数变薄包装。
- `src-tauri/src/chat/agent/loop_.rs`（或 rounds.rs）：每轮 `run_tool_round` 后、下一 `planning_step` 前，若 `activated_allowed_tools` 非空且**自上次应用有变化**，`retain_tools_for_allowed(&mut state.tools, &snapshot)`（track len/hash 仅变更时应用）。**仅单调收窄**（只 retain 不 re-expand），与 Plan-mode 过滤可组合、顺序无关。代码注释标注 intended。

### T4 skill_read_file 上限

- `src-tauri/src/skills/runtime.rs::read_skill_file`（120-126）：复用 `native_tools::MAX_READ_FILE_BYTES`（files.rs:129；如未 `pub` 则补 `pub use`）。`fs::read` → 超限用 `from_utf8_lossy` 取 head + 截断标记（提示用 `skill_run_script` 处理全文），UTF-8 边界安全。`read_file_with_cache`（runtime.rs:38-51）缓存的即截断后内容，无需改缓存方法。

## 测试计划

**后端单测**（`cargo test --manifest-path src-tauri/Cargo.toml`）：
- `substitute_arguments_replaces_full_and_positional` / `..._missing_positional_is_empty`
- `read_skill_file_caps_oversize` / `..._returns_full_when_small`
- `skill_run_cache_builds_registry_once`
- `find_by_trigger_matches_explicit` / `..._default_is_slash_id` / `..._requires_leading_slash`
- `parse_skill_markdown_reads_triggers_and_arguments`
- `retain_tools_for_allowed_keeps_skill_and_builtins` / `..._noop_when_empty`
- 注意更新 runtime.rs 测试里 5 处穷举构造的 `SkillMeta {}` 字面量（加 3 字段）。

**前端**：无测试 runner（CLAUDE.md）。优先抽纯 helper 到 `src/chat/slashCommands.ts`（`buildSlashCommands`/`matchSlashSkill`）+ vitest；若超范围则靠后端 `try_apply_skill_slash_trigger` 集成测 + 手测弹层。

**手测**：建 `~/.../skills/commit/SKILL.md`（`triggers:[/commit]`、`arguments:[message]`、body 含 `$ARGUMENTS`），`/commit fix login` → 弹层显示 `/commit`、Enter 发送、pin skill、body 渲染 `fix login`；`/help` 仍开命令列表。**红线**：providers/API key/skill 配置全程不动。

## Commit 切分（各步独立可编译 + 测试）

1. `feat(skills): cap skill_read_file size`（T4，最小隔离）。
2. `feat(skills): run-level registry cache in SkillRunCache`（T1，纯性能，无行为变更）。
3. `feat(skills): frontmatter triggers/argument-hint/arguments + find_by_trigger + substitute_arguments`（T2 核心，inert）。
4. `feat(chat): slash skill trigger via chat_send_message preprocessing`（T2 后端接线，后端即可用）。
5. `feat(chat): dynamic allowed_tools filtering for model-activated skills`（T3，最 invasive）。
6. `feat(chat): surface skills in slash command menu`（T2 前端，可最后/并行）。

顺序 1→2 先行（低风险即时价值），3→4 交付头条验收，5 最 invasive，6 纯 UI。

## 关键风险

- **`call_skill_tool` 借用检查**：cached `&registry` 被 `&mut cache` 持有，派发前必 `.clone()` 出 `SkillRecord`。
- **T3 单调收窄语义**：activate A（窄）后再期望 B 更宽的工具不会被 re-add——这是 intended「作用域收紧」，代码注释说明。
- **trigger 与内置冲突**：skill id 为 `help`/`new` 等会在后端 trigger 路径遮蔽内置，但内置 `/help` 被前端弹层在发送前拦截、不到 `chat_send_message`，低风险；命中内置名时加 debug log。
- **`disable_model_invocation` skill** 仍应可 slash 触发（显式用户调用）；`find_by_trigger` 忽略该 flag（该 flag 只 gate 模型自动调用），唯一 gate 是 `is_skill_enabled`。
