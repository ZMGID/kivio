---
status: accepted
---

# 低频工具按需加载

2026-11-09，用户授权主代理在工具瘦身任务中自行决定细节。本文决定内置运行时如何减少每次请求携带的工具定义。前置工作已完成：基线测试（`src-tauri/src/chat/commands/tool_baseline_tests.rs`）、自动化与定时任务工具合并、无知识库时隐藏 `knowledge_search`、描述压缩与工具顺序稳定。

参考实现：oh-my-pi 的 `xd://` 设备（`packages/coding-agent/src/tools/xdev.ts`、`essential-tools.ts`，2026-11-08 快照）；OpenCode 的 Code Mode `search`（MCP 默认不进主列表）；Pi 默认只给 `read`/`bash`/`edit`/`write`。

## 决定

内置工具分为**常驻**和**按需**两类，类别写在 `NativeToolEntry` 上（`load_mode`），是唯一来源。

- 常驻工具照旧进入模型工具列表。
- 按需工具不进入工具列表，挂在 `tool://` 下，通过已有的 `read` / `write` 使用：
  - `read tool://` 列出当前会话可用的按需工具（名称 + 一行说明）。
  - `read tool://<name>` 返回该工具的完整说明和参数 schema。
  - `write tool://<name>`，`content` 为 JSON 参数，执行该工具。
- 系统提示只放按需工具的目录（每个一行），不内联 schema。

Kivio 一轮运行中途不能新增工具定义，所以只能复用已有工具做传输，不能“发现后再挂载”。`read` 已支持 `mcp://`，`write` 已支持 `agent://`，`tool://` 沿用同一模式。

### 常驻与按需的划分

常驻：`read`、`write`、`edit`、`bash`、`grep`、`glob`、`skill`、`ask_user`、`todo_write`、`task`、`wait`、`present_artifacts`、`web_search`、`web_fetch`、`save_plan`、Goal 工具、`yield`。

- `read`/`write` 是传输本身，不能按需。
- `web_search`、`ask_user`、`todo_write`、`grep` 固定常驻：OMP issue #5973 证明模型不会通过设备协议调用网页搜索；交互和清单工具与宿主集成，隐藏后失去作用。
- Goal 与 `save_plan` 由会话状态决定是否出现，已经按模式追加，不再加一层。

按需：`automation`、`schedule`、`kivio_inspect`、`kivio_configure`、`html_render`、`html_preview`、`bash_output`、`kill_background`、`eval`、`memory_*`、`advisor`、`knowledge_search`。新增内置工具默认按需，需要常驻须说明理由并更新本文。

MCP 工具本决定不改变：仍按服务器配置进入工具列表。是否将 MCP 也改为按需另行决定。

## 约束

1. **授权不降级。** `write tool://<name>` 执行时，审批、会话授权、plan/chat 只读限制、按 action 的只读判断、超时和并行策略全部按**被调用工具**的元数据判断，不按 `write`。同一 `before_tool_call` 入口处理两种调用方式。
2. **可见即可用。** `read tool://` 只列出本轮实际可执行的按需工具，经过与常驻工具相同的过滤（设置开关、模式过滤、助手限制、子代理角色白名单、工作流禁用）。未列出的名字执行时返回与 `disabled_builtin_tool_feedback` 相同的提示。
3. **直接调用兼容。** 模型按旧习惯直接调用按需工具名时，仍能找到并执行，等价于 `write tool://<name>`；历史记录中的工具卡片照常显示。
4. **子代理。** 角色白名单写的是内部工具名，不写 `tool://`。只读子代理（scout、seance、plan）可以 `read tool://`，但只能执行只读动作。
5. **展示。** 工具卡片显示被调用工具本身（名称、参数、结果），不显示为一次文件写入。
6. **开关。** 设置中保留“低频工具按需加载”开关，默认开启；关闭时所有可用工具回到工具列表，用于对照和排障。

## 取舍

- 接受首次使用按需工具多一次 `read` 的往返；参数错误时直接返回 schema，避免再读一次。
- 接受模型需要学会 `tool://` 协议；目录写进系统提示，且直接调用按需工具名仍然有效，降低误用成本。
- 不采用 OpenCode 的 `execute` 脚本方案：需要新的运行时与沙箱，超出本次目标。

## 验收

- 基线测试中 Agent 模式工具数降到常驻集合，Tools token 相对 ec86706a 明显下降，并记录数字。
- 测试覆盖：目录过滤与常驻集合一致；按需工具经 `write tool://` 与直接调用走同一审批与只读判断；plan 模式拒绝写类动作；未授权工具不出现在目录且执行被拒；开关关闭时恢复全量列表。
- 用真实模型验证至少一次 `read tool://` → `write tool://automation`（action=list）完整调用。未执行的验证须在交付说明中明示。
