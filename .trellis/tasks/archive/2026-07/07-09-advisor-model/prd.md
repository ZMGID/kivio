# Advisor 顾问模型

## Goal

实现 executor-advisor 模式（Anthropic 推荐用法的反向组合）：便宜/快速模型跑主循环（executor），遇到困难时通过 `advisor` 工具调用一个更强的模型获取指导（advisor, on-demand）。大多数 token 按 executor 费率计费，贵模型只在被咨询时产生消耗。

## Background（代码现状）

- 原生工具注册表 `mcp/native_registry.rs::NATIVE_TOOLS`（entry: def/enabled/parallel_safe/call），工具定义在 `mcp/types.rs`（`native_*_tool()` 返回 `ChatToolDefinition`）。
- 单次模型调用（不进 agent loop）的现成范例：`commands.rs::generate_title_with_model` → `call_chat_completion_message` → `generate_with_chat_provider`（四协议适配、usage 记账、失败重试齐全）。
- 混音器设置：`DefaultModelsConfig`（settings.rs ~611：chat/vision/title_summary/compression/image_generation 五个 `DefaultModelSelection`），有 `sanitize_default_model_selection` 校验；UI 在 `SettingsShell.tsx` 混音器区块（`ModelPairSelect` + `updateDefaultModel`）。
- 子代理工具表由 `filter_tools_for_agent` 过滤，advisor 工具对子代理的暴露需要决策（见 Requirements 6）。

## Requirements

1. **设置字段**：`default_models.advisor`（`DefaultModelSelection`，providerId+model），默认空 = advisor 功能关闭（不注册工具）。走 `sanitize_default_model_selection` 同款校验。
2. **`advisor` 原生工具**：注册进 `NATIVE_TOOLS`。schema：`question`（必填，要咨询的问题）+ `context`（可选，相关代码/背景/已尝试的方案）。read_only、parallel_safe、不需要 session consent、不 sensitive。
3. **暴露条件**（enabled 门）：advisor 模型已配置且 provider 可用。未配置 = 工具完全不出现在模型工具表（不是报错）。
4. **调用实现**：走 `call_chat_completion_message` 同款单次调用（非 agent loop、无工具、不递归）：system prompt 定位为「资深顾问：给出诊断和方向性指导，不代写完整实现」，user 内容 = question + context。usage 记账 label 用 "Advisor consultation"，模型/provider 归属正确。
5. **防滥用**：advisor 工具描述中写明适用场景（卡住/反复失败/重大方案抉择时咨询），避免 executor 事事咨询；advisor 回复长度上限用模型默认（不特殊限制）。
6. **子代理也可用**：worker 卡住时正是 advisor 的核心场景（图中 executor 就是 worker 角色），`filter_tools_for_agent` 不剥离 advisor（与 `agent` 工具不同，advisor 无递归风险——它是单次调用）。
7. **设置 UI**：混音器加「Advisor 模型」行（`ModelPairSelect`），inheritLabel 为「不启用 / Off」语义（空 = 关闭功能，与生图模型的「未配置则禁用」同款措辞风格）；提示文案说明「配置一个更强的模型作为顾问，主模型遇到困难时可主动咨询」。「重置混音器模型」一并清空。
8. **系统提示**：advisor 工具可用时，在系统提示 segments 里加一句轻量引导（何时该咨询顾问），与 todo/ask_user 的 format_prompt 模式一致。

## Acceptance Criteria

- [ ] 未配置 advisor 模型：工具表中无 `advisor`，行为与现状完全一致；现有测试全绿。
- [ ] 配置后：模型工具表出现 `advisor`；调用它会以配置的 provider+model 发起单次补全并返回建议文本；usage 日志归属 advisor 模型。
- [ ] advisor 调用失败（key 失效/网络错误）：返回工具错误文本，主循环继续，不中断生成。
- [ ] 子代理工具表包含 advisor（配置时），子代理内调用同样生效。
- [ ] 配置的 provider 被删除：sanitize 重置字段，工具随之消失，无 panic。
- [ ] 混音器 UI 可配置/清除；`npm run lint` + `npm run typecheck` + 相关 Rust 测试通过。
- [ ] probe 实测：executor（便宜模型）在 prompt 引导下调用 advisor，usage 中出现 advisor 模型的单次调用记录。

## Non-Goals

- advisor 不进 agent loop、不带工具、不递归咨询（单次 Q→A）。
- 不做多轮顾问对话/顾问记忆（后续有需要再说）。
- 不做 per-agent-definition 的 advisor 覆盖。
