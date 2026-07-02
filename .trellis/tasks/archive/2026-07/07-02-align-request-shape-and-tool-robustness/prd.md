# 对齐 opencode 请求规范：稳定系统提示词与工具调用健壮性

## Goal

基于 opencode 真实流量对比（`E:\ZM database\kop\trace_opencode_2026-07-02.json`，16 轮 agent 会话）修复 Kivio 三个已证实的差距。调研结论（journal session 2026-07-02）：opencode 的系统提示词 15 轮 agent 请求逐字不变（日期只到天），而 Kivio 注入分钟级时钟导致每轮前缀都变——打穿正经供应商的 prompt cache、也让会话亲和型代理无法续会话；opencode 对模型返回的坏工具调用（大写 `Grep`/`Read`、错误参数名——trace 中 16 轮出现 2 次）大小写不敏感匹配 + 校验错误喂回模型自愈，Kivio 则硬报错断轮。

## Requirements

### R1：系统提示词稳定化（前缀一天内逐字节不变）
- `chat_current_datetime_context`（settings.rs:2093）从"年月日 星期 HH:MM"降为"年月日 星期"（opencode 同款：`Today's date: Thu Jul 02 2026`）。移除时分。
- 排查 `build_chat_system_prompt_with_segments`（prepare.rs）全部注入段：确认除日期外没有其他逐轮变化的内容（如随机 id、每轮变化的 todo/plan 状态段是业务必需、保持，但需确认它们在会话内未变时字节稳定）。
- 目标：同一对话内容不变时，相邻两轮请求的 system 消息字节级相同。

### R2：工具调用健壮性（opencode 自愈模式）
- **大小写/风格不敏感匹配**：`match_tool_call`（execute.rs:56）在精确匹配失败后，做 case-insensitive 匹配（`Grep`→`grep`、`Read`→`read`、`Bash`→`bash`）。仅当不区分大小写后唯一命中才采用；多义或仍无命中走 R2b。
- **未知工具喂回自愈**：`unknown_or_disabled_tool_result`（rounds.rs:577）的错误文案从裸 `Unknown tool requested: X` 改为指导性内容：`Unknown tool: X. Available tools: <declared tool names>. Please call one of the declared tools.`——模型下一轮可自我纠正（错误仍作为 tool result 返回，本来就会继续循环，只是缺可用工具清单）。
- **参数校验错误喂回**：确认参数解析失败路径（`arguments_parse_error`，rounds.rs:232）返回的错误文案含具体缺失字段与"please rewrite"指导（对齐 opencode 的 `SchemaError(Missing key at ["filePath"]). Please rewrite...`）。若已有则不动。

### R3：planning 空响应自动重试一次
- `finalize_planning_final` 走到 `final_response_from_planning_message` 报 empty 时（loop_.rs:243 / stop.rs:71），或 planning 无工具调用且正文为空时：不立即失败，**原地重试一次 planning 调用**（同参数）；重试仍空才报既有错误。
- 约束：只重试一次（防雪崩）；重试前检查取消；microcompact/摘要等状态不受影响。

## Acceptance Criteria

- [x] AC1：同一对话连续两轮（间隔 >1 分钟）发送，system 消息字节级一致（单测：mock 两次构造对比；时间只含日期）。
- [x] AC2：模型返回 `Grep`/`Read`/`Bash`（大写）时匹配到 `grep`/`read`/`bash` 并正常执行（单测覆盖大小写命中、多义不采用、无命中走未知工具路径）。
- [x] AC3：未知工具的 tool result 含声明工具清单和纠正指导；轮次不中断（既有行为，文案增强）。
- [x] AC4：planning 空响应第一次自动重试、第二次才报错（单测/harness 覆盖）。
- [x] AC5：`npm run typecheck`/`lint`/`test` 全绿；`cargo check --lib --tests` 干净；新逻辑有单测（cargo test 环境问题时用 harness 验证并注明）。
- [x] AC6：实测（用户）：冰供应商 grok-composer-2.5-fast 跑带工具任务，工具调用可用、无卡死；正经供应商多轮任务 cached tokens 明显上升。

## Notes

- 范围外：DSML 工具路径、外部 agent、Anthropic adapter 的 cache_control 显式标记（另行任务）。
- 风险：R2a 大小写匹配需防误伤——MCP 服务器可能真有同名不同大小写的工具，故"唯一命中才采用"。
- trace 文件是用户导出的敏感数据，不入库、不引用其内容细节到代码注释。
