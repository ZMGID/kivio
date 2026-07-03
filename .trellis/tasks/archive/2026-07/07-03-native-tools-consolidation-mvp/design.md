# Design：原生工具集精简（MVP）

前置：读 prd.md + research/（tool-inventory / consolidation-touchpoints / risks-and-open-questions）。

## 1. 总体思路

4 项改动共享同一个**核心机制**：一张**旧名归一化表**（legacy alias），让被移除/改名的旧工具名在两条路径上都能解析到新工具——① 模型发出的工具调用，② persona/skill 存储的工具白名单。其余是各改动局部的 def 移除 + handler 调整 + 提示词/测试同步。

注册表 `mcp/native_registry.rs::NATIVE_TOOLS` 是单一真源：移除/改名条目后，per-round `base_tools`（`loop_.rs`）自动传播，一组 `#[cfg(test)]` 快照测试是回归护栏。

## 2. 核心：旧名归一化表（区别于 wire alias）

### 与现有 wire alias 的区别（关键，别混用）
- `RESERVED_WIRE_ALIASES`（`types.rs:11`，`web_search`→`search_web`）：改**模型可见名**，内部 `tool.name` 不变。方向 = 内部名→线上名。
- 本次需要的是**反向**：新名（`glob`/`read`/…）成为真实 `tool.name`，**旧名（`find`/`ls`/…）作为输入被解析回新名**。这是"输入归一化"，不是 wire alias，**不能复用** `RESERVED_WIRE_ALIASES`。

### 新增归一化表
在 `mcp/types.rs`（与 wire alias 相邻，便于对照）新增：

```rust
/// 旧工具名 → 现工具名。用于移除/改名后，旧名仍能路由（模型调用 + 存储白名单）。
/// 与 RESERVED_WIRE_ALIASES 方向相反：这里旧名是“输入”，被规整到现名。
pub const LEGACY_TOOL_ALIASES: &[(&str, &str)] = &[
    ("ls", "read"),
    ("find", "glob"),
    ("list_background", "bash_output"),
    ("todo_update", "todo_write"),
];

pub fn canonical_tool_name(name: &str) -> &str { /* 命中返回新名，否则原样 */ }
```

### 两处消费点
1. **模型调用**：`chat/agent/execute.rs::match_tool_call`（execute.rs:56）在现有 `openai_tool_name()==fn || tool.name==fn` + 大小写兜底之外，追加"把 `function_name` 过一遍 `canonical_tool_name` 再比一次"。旧名调用 → 命中新工具。
2. **存储白名单**：`chat/agent/prepare.rs::tool_matches_recommended_name`（prepare.rs:713）在比对前，把 recommended `name` 过 `canonical_tool_name`，使 persona 里写 `find`/`ls`/`todo_update` 仍匹配到 `glob`/`read`/`todo_write`。

> 说明：`list_background`→`bash_output` 放进表主要是让模型的旧调用不落空（会命中 `bash_output` 定义并按"无 job_id=列表"语义工作）；persona 不引用它，兼容压力低。

### 提示词不变量
`native_tools_prompt` 仍走 `apply_reserved_wire_alias`（wire 层），归一化表**不参与**提示词渲染——模型看到的就是新名 `read/glob/bash_output/todo_write`，旧名仅作输入兼容。

## 3. 各改动契约

### R1：`read` 读目录 + 移除 `ls`
- **handler**（`native_tools/files.rs`）：`call_read_file`（native_registry.rs:392）在 image / PDF・Word・Excel 分支之后、落到 `read_file` 之前，增加 `full.is_dir()` 分支 → 调用 `list_dir` 现有逻辑，返回其 `{path, entries, truncated}` JSON。保持 image/doc 分支不变。
- **offset/limit 对目录**：忽略（不分页），仅文件读用。`include_hidden`/`max_entries` 目录场景不在 `read` schema 里，用 `list_dir` 默认（隐藏不含、200 上限）——如需可后续加，MVP 从简（PRD Notes 已注明可能精简元数据）。
- **def**：`types.rs` 更新 `native_read_file_tool` 描述"文件按行号读、目录返回条目列表"；移除 `native_list_dir_tool`。
- **registry**：移除 `ls` 条目（native_registry.rs:159）。
- **兼容**：`ls` 入归一化表 → 旧调用/persona 白名单命中 `read`。

### R2：移除 `list_background`，并入 `bash_output`
- **schema**（`types.rs` `native_bash_output_tool`）：`job_id` 由 required 改为可选；描述补"不传 job_id 时返回本会话所有后台作业列表"。
- **handler**（`native_tools/shell.rs` + `native_registry.rs::call_bash_output`）：无 `job_id` → 走原 `list_background` 逻辑返回作业列表；有 `job_id` → 原增量读不变。`list_background` fn 保留为内部实现被 `bash_output` 复用（不必删函数，只删工具入口）。
- **def/registry**：移除 `native_list_background_tool` + `list_background` 注册条目。
- **兼容**：旧名入归一化表 → 模型调 `list_background` 命中 `bash_output`（无 job_id 分支）。

### R3：`todo_update` 并入 `todo_write`，移除 `todo_update`
- 研究确认 `todo_update` 无 `todo_write` 无法表达的**状态**能力（整表替换覆盖增删改；悬挂依赖边由 `sync_dependency_edges` 清理）。仅损失"changed 回执精确字段名"这一便利，可接受。
- **todo.rs**：移除 `todo_update_tool`/`TodoUpdateArgs`/`apply_todo_update`/`TODO_UPDATE_TOOL_NAME`；`apply_tool`/`tool_definitions`/`is_agent_todo_tool_name` 简化为单工具；`format_prompt` 去掉 `todo_update` 措辞。
- **registry**：移除 `todo_update` 条目。
- **兼容**：`todo_update` 入归一化表 → 命中 `todo_write`（模型需改传整表；这是可接受的行为变化，已在 AC4/AC5 覆盖）。

### R4：`find` → `glob`
- **def**：`native_glob_files_tool` name `"find"`→`"glob"`（id `native__glob_files` 不变）。
- **registry**：条目 name `"find"`→`"glob"`。
- **prepare.rs**：`action_examples` 的 `"read"|"ls"|"grep"|"find"` → 含 `glob`（去 `ls`、`find`→`glob`）。
- **内置 persona**（`agents/types.rs`）：直接把 `"find"`/`"ls"` 改成 `"glob"`/`"read"`（源码可改）；用户/项目 `.md` persona 靠归一化表兜底。
- **兼容**：`find` 入归一化表。

## 4. 数据流（改动后一次工具调用）

模型发 `find` 调用 → `match_tool_call`：直接比不中 → 过 `canonical_tool_name("find")="glob"` 再比 → 命中 `glob` 工具 → `find_entry("glob")` → `glob_files` handler。persona 白名单同理经 `tool_matches_recommended_name` 归一化匹配。

## 5. 兼容性 / 回滚

- **回滚点**：归一化表 + 各改动彼此独立，可逐项 revert。表本身是纯新增，最先落、最后依赖。
- **向后兼容**：旧名（模型侧 + persona 侧）全覆盖，AC5/AC6 验证。会话是否持久化工具名列表 → 实现首步核实 `chat/storage.rs`（研究判断否，但需确认；若有则同样过归一化）。
- **前端**：`ChatNativeToolsConfig` 开关不动（合并的都同组 toggle）。

## 6. 测试策略

- **注册表快照测试**（`native_registry.rs`：EXPECTED_ORDER / session_consent_set / parallel_safe_set / read_only_set / builtin_exposure_snapshot；`types.rs`：default_native_config_exposes / file_tool_path_descriptions；`files.rs`：simulated_agent_session "Pi's 7"）：按新工具集更新期望列表。
- **新增单测**：`canonical_tool_name` 映射；`match_tool_call` 对旧名（find/ls/todo_update/list_background）命中新工具；`tool_matches_recommended_name` 对 persona 旧名命中；`bash_output` 无 job_id 返回列表；`read` 目录分支返回 entries；`todo_write` 覆盖单项增删改。
- **改写**：`execute.rs` 现有 `match_tool_call(&tools,"Glob").is_none()`（execute.rs:874，今日无 Glob）→ 现在应命中；todo.rs 三个 `apply_todo_update` 单测改写为 `todo_write`；`stop.rs` DSML 用例不涉本次（skill 未动）。
- **环境**：cargo test 若遇 0xC0000139，用 harness 二进制验证（沿用本仓库既有做法）。
- 前端 `npm run typecheck/lint/test` 全绿（本次基本不动前端，回归确认）。

## 7. 不做（二期）

skill 三件套合并（安全约束回退，单独设计）、memory 三合一、bash 后台流式重构、`edit` 的 replaceAll、`read` 目录元数据（大小/隐藏/分页）增强。
