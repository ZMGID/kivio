# Simplify Native File Tools

## Goal

把 Kivio Chat 的文件写入/变更类内置工具从 11 个收敛到成熟编码 agent 的极简形态（参考 Claude Code / pi agent / Codex）。删除分段写入草稿协议（begin/append/finish/abort_file_write + write_file_chunk）。复杂保护（原子写、路径锁、BOM/行尾保留、占位符拦截）全部下沉到 runtime，对模型不可见。模型看到的工具要"像人话"：少而语义强，描述一句话说清，不再需要 2000 字系统提示教学。

## 背景（Why now）

- 分段写入协议的原始动机是全局 HTTP 60s 超时导致大文件 content 参数流式生成中途断流。该根因已被 `88f76ae`（split streaming HTTP timeout handling）修复，协议残余价值大幅缩水。
- 协议成本是常驻的：4 个模型可见工具 + 幻觉诱饵参数（expected_sha256/expected_bytes/chunk_sha256）+ 系统提示里近 2000 字教学文本 + 草稿目录无 GC 泄漏。
- 主流编码 agent（Claude Code: Write/Edit；Codex: apply_patch；OpenCode: write/edit/patch；pi agent: 极简 ~6 工具）没有任何一家把大文件保护做成模型可见的多工具协议。

## What I already know

- 工具定义在 `src-tauri/src/mcp/types.rs`（`list_native_builtin_tool_defs` 按设置开关组装）。
- 实现在 `src-tauri/src/native_tools/files.rs`（~1900 行，其中草稿会话相关 begin/append/finish/abort/write_file_chunk + session 辅助函数约 600+ 行）。
- 分发在 `src-tauri/src/mcp/registry.rs:528-556`（file mutation 工具走 spawn_blocking）。
- 系统提示教学在 `src-tauri/src/chat/agent/prepare.rs:822`（zh）/ `:831`（en），单段近 2000 字。
- 禁用工具反馈名单在 `prepare.rs:158-187`（BUILTIN_NAMES）。
- 前端渲染在 `src/chat/ToolCallBlock.tsx:260-264, 502-520`（含全部 session 工具分支）。
- inline-code 请求过滤在 `src-tauri/src/chat/commands.rs`（apply_inline_code_request_tool_filter，retain 名单含全部 session 工具名）。
- `write_file_chunk` 工具定义从未出现在 `list_native_builtin_tool_defs` 暴露列表中（模型从未见过），dispatch 兼容分支可安全删除——工具调用不会从历史会话重放。
- runtime 保护已实现且应保留：`atomic_write_text/bytes`（同目录临时文件+rename）、`acquire_file_mutation_locks`（路径锁）、BOM/CRLF 保留、`assert_writable_path` 黑名单、项目根目录约束。
- `looks_like_placeholder_content` 对中文散文有误伤面（"省略""保持不变"为日常用语）。
- `read_file` 对 >2MB 文件直接报错，即使带 offset/limit 也不让读（files.rs:171）——成熟 agent 是窗口化读取。
- 测试覆盖：`loop_.rs`、`stream.rs`、`prepare.rs`、`types.rs`、`files.rs` 内大量引用 session 工具的单测需同步清理。

## Assumptions (temporary)

- 88f76ae 的流式超时修复已让"巨型参数断流"降为可接受的小概率事件，失败即重试（与 Claude Code 同一立场）。
- 历史会话只做展示不重放工具调用，删除 dispatch 分支不破坏旧会话打开。

## Open Questions

（已全部决策，见 Decision (ADR-lite)）

## Requirements (evolving)

- 删除 `begin_file_write` / `append_file_write` / `finish_file_write` / `abort_file_write` / `write_file_chunk`：工具定义、registry dispatch、files.rs 实现 + session 辅助函数、草稿存储目录逻辑、BUILTIN_NAMES 条目、inline-code 过滤名单、相关测试。
- **删除 `patch` 工具**：工具定义、dispatch、`parse_patch`/`apply_patch_hunks`/`validate_patch_path` 等 ~200 行格式解析实现、相关测试、提示词与过滤名单引用。多文件改动 = 多次 edit_file（并行执行已支持）。
- 写侧终态：`write_file(path, content)` + `edit_file(path, old_string, new_string, replace_all?)` 双工具，与 Claude Code / pi agent 一致；目录管理 `create_dir/delete_path/move_path/copy_path` 保留不动。
- 保留全部 runtime 保护且对模型不可见：原子写（同目录临时文件+rename）、路径锁、BOM/CRLF 保留、`assert_writable_path` 黑名单、项目根目录约束。
- 工具描述全部重写为 1-2 句人话，删除死引用、删除 300 行/20KiB 分流规则、删除幻觉诱饵参数（expected_sha256/expected_bytes/chunk_sha256 随协议一并消失）。
- **系统提示内置工具段（prepare.rs zh/en 两份）全段重写**：分行结构化、每工具一句话语义，文件工具教学压缩为一句优先级规则（小改 edit_file / 新建或整文件覆盖 write_file）；run_python/run_command/memory 语义保留但重写紧凑。目标 ≤ 现有 1/3。
- **read_file 窗口化**：带 offset/limit 时允许读 >2MB 文件（按行窗口读取，不整文件载入）；不带时维持 2MB 上限报错并在错误信息中提示用 offset/limit。
- `looks_like_placeholder_content` 收敛误伤面：仅对覆盖已存在的代码类文件启用（新文件、散文/文档类不拦截）。
- 前端 ToolCallBlock.tsx：保留旧工具名（session 工具、patch、write_file_chunk）的历史会话渲染分支（只读展示），从活跃路径移除。
- 更新 `.trellis/spec/backend/file-tools.md` 反映新工具面。

## Acceptance Criteria (evolving)

- [x] `list_native_builtin_tool_defs` 写侧恰为 6 个（write_file、edit_file、create_dir、delete_path、move_path、copy_path），无 patch、无任何 session 工具（types.rs `write_gate_exposes_exactly_whole_file_and_path_tools` 测试锁定）。
- [x] 全仓库无活跃引用：Rust 侧仅剩 types.rs 测试中的 removed 断言名单；前端仅 ToolCallBlock 历史渲染分支（带 legacy 注释）。dispatch、BUILTIN_NAMES、inline 过滤名单、approval summary、stream 预览均已清理。
- [x] 系统提示内置工具段：zh 1917→639 字符（33%）、en 4162→1325 字符（32%），均 ≤ 1/3 且分行结构化。
- [x] write_file 无大小分流（300行/20KiB 规则随协议删除，无任何残留提示）。
- [x] read_file 带 offset/limit 可流式读取 >2MB 文件行窗口（`read_file_rejects_oversized_file_without_window_but_allows_offset_limit` 测试）；不带时报错信息含 offset/limit 提示。
- [x] 占位符拦截收敛：新文件、散文类（.md/.txt 等非代码扩展名）不拦截，覆盖已存在代码文件仍拦截（`write_file_allows_placeholder_phrases_in_new_and_prose_files` 测试）。
- [x] `cargo test` 280 全绿；`npm run typecheck` / `npm run lint` 通过。
- [ ] 手工冒烟（待用户运行 app）：项目会话内 write_file 新建、edit_file 小改、delete_path 删除；打开含旧 session 工具调用的历史会话确认渲染不报错。

## Definition of Done

- Tests added/updated（删除 session 工具测试，补 write_file 大内容 + placeholder 收敛用例）
- Lint / typecheck / cargo test green
- `.trellis/spec/backend/file-tools.md` 与 index.md 更新
- 不动用户 settings 结构（native_tools.write_file/edit_file 开关语义不变）

## Out of Scope (explicit)

- run_command / run_python / memory / skill 工具的**行为**变更（提示词重写只压缩表述，不改变这些工具的规则语义）
- 设置 UI 变更；settings 结构不动（native_tools.write_file/edit_file 开关语义不变，edit_file 开关现在只控制 edit_file 一个工具）
- 读侧新增工具
- 重新引入任何形式的可恢复写入协议或多文件补丁格式

## Decision (ADR-lite)

**Context**: 分段写入协议（4 个 session 工具 + 幻觉诱饵参数 + 2000 字提示教学）源于把"60s 全局 HTTP 超时导致大参数流断"误归因为写入工具问题。根因已在 `88f76ae` 修复。成熟编码 agent（Claude Code / pi agent / Codex / OpenCode）无一暴露模型可见的大文件保护协议。

**Decision**（2026-06-11，zhimeng 确认）：
1. 删除整套分段写入协议（begin/append/finish/abort_file_write + write_file_chunk），不做任何兼容保留。
2. 删除 patch 工具——Kivio 是桌面助手非仓库级编码 agent，V4A 格式对任意 OpenAI 兼容模型遵从性不可靠；写侧终态 = write_file + edit_file。
3. 系统提示内置工具段全段重写（zh/en），目标 ≤ 1/3，分行结构化。
4. 顺带修 read_file：带 offset/limit 时窗口化读取任意大小文件。
5. 设计原则固化：**模型工具面少而语义强、像人话；复杂保护（原子写/锁/BOM/占位符拦截）全部沉到 runtime，不暴露为工具或参数。**

**Consequences**:
- 极小概率的"超长生成中途断流"恢复能力放弃——失败即重试，与 Claude Code 同一立场。
- GPT/Codex 系模型失去 apply_patch 训练加成，多文件编辑改为多次 edit_file。
- files.rs 预计净删 800+ 行；提示词 token 每轮节省约 2/3。
- 旧会话中的 session/patch 工具调用记录仅由前端历史分支渲染，后端不再认识这些名字。

## Technical Notes

- 上个任务研究材料：`.trellis/tasks/06-10-agent-file-editing-workflow/research/agent-file-editing-patterns.md`（Codex/OpenCode/Aider/Claude text editor/Hermes 模式综述，2026-06-10 二次核实）。
- 本任务补充研究：`research/`（pi agent + Claude Code 当前工具面一手核实，进行中）。
- 旧 PRD `.trellis/tasks/06-10-agent-file-editing-workflow/prd.md` 的"draft-session layer"立场被本任务明确推翻：根因（60s 全局超时）已在 HTTP 层修复，保护协议不应暴露为模型工具面。
- 草稿目录 `DRAFT_ROOT_DIR` 全仓库无 GC 调用——删除协议后整个目录逻辑一并移除，无需补 GC。

## Research Notes

### 成熟 agent 写侧工具面（待 research 子代理核实细节）

| Agent | 写侧工具 | 备注 |
|---|---|---|
| Claude Code | Write、Edit | 大文件也一次 Write，无分段协议 |
| pi agent | write、edit | 总工具面 ~6 个 |
| Codex CLI | apply_patch | OpenAI 模型对该格式有 RL 训练 |
| OpenCode | write、edit、patch | 三者并存但同一权限组 |

### patch 删留的初步分析（Q1 推荐：删）

- 删：Kivio 是桌面助手不是仓库级编码 agent，多文件协同编辑场景罕见；V4A 格式对任意 OpenAI 兼容模型（Qwen/GLM 等）的格式遵从性不可靠，弱模型易产废 hunk；parse_patch + apply_patch_hunks ~200 行是最脆的格式依赖；Claude Code 仅靠 Edit+Write 活得很好，多文件 = 多次 edit_file 调用（并行分发已支持）。
- 留：Codex 系模型对 apply_patch 有训练加成；已实现且有测试。
