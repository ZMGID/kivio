# 显式文件展示通道

## Goal

把“文件被工具生成”与“文件需要展示给用户”彻底分开。`write`、`run_python` 和 MCP 工具只登记产物，不再因为产生文件就自动在回答末尾显示；模型只有明确调用展示工具时，文件或图片才进入聊天内容，并按调用发生的位置显示。

## Background

当前 `ChatMessage.tool_calls[].artifacts` 会被前端统一收集，图片进入回答下方画廊，非图片进入回答末尾文件卡。因此临时文件、中间结果和模型并不打算交付的文件也会无条件出现。文件卡只能位于回答末尾，无法像正文一样控制位置。

现有消息时间线已经用 `ChatMessage.segments` 保存文本、推理和工具调用的顺序。展示工具本身可以作为稳定的位置锚点，不需要用文件路径、Markdown 文件名或字符偏移决定位置。

## Requirements

1. 新增原生工具 `present_artifacts`，专门表达“把这些已产生的 artifact 展示给用户”。
2. 工具只接受 Kivio 分配的 artifact ID，不接受文件路径、URL 或任意文件系统位置。
3. 每个新产生的工具 artifact 获得稳定 ID，并在对应工具结果中告知模型可用 ID。
4. `write`、`run_python`、MCP 工具继续生成和保存 artifact，但新 artifact 未被 `present_artifacts` 引用时不得自动显示。
5. `present_artifacts` 的工具调用 segment 作为展示位置：模型可先输出正文、调用展示工具、再继续正文，从而把图片或文件卡放在回答中间。
6. 图片和普通文件均支持显式展示；可选 caption 作为展示说明。
7. 引用只能解析当前 assistant 消息内工具调用产生的 artifact；无效、跨消息或不存在的 ID 显示受控不可用提示，不按路径回退。
8. 同一 artifact 默认只展示一次；重复 ID 去重。
9. 旧消息保持兼容：没有 artifact ID 的历史 artifact 继续按原规则在回答末尾显示，旧 Markdown 图片引用继续可用。
10. 展示工具无需审批，不读取或修改文件，只选择当前消息已登记的 artifact。
11. 更新工作台和 `run_python` 提示，明确“生成不等于展示”，需要交付时调用 `present_artifacts`。

## Acceptance Criteria

- [x] `write` 或 `run_python` 产生文件后，如果模型没有调用 `present_artifacts`，聊天中不出现新文件卡或图片画廊。
- [x] 模型调用 `present_artifacts` 后，指定图片或文件卡显示在该工具调用对应的时间线位置。
- [x] 回答可以形成“正文 → 图片/文件 → 后续正文”的稳定顺序，重载对话后顺序不变。
- [x] 工具结果向模型返回 artifact ID 和名称，但不要求模型传递本地路径。
- [x] 任意伪造、跨消息或不存在的 artifact ID 不会加载本地路径或 URL。
- [x] 历史无 ID artifact 仍按旧逻辑显示。
- [x] TypeScript 类型检查、lint、Rust check 和相关单元测试通过。

## Out of Scope

- 不允许模型通过展示工具直接展示任意路径文件；文件必须先由已有工具登记为 artifact。
- 不在本阶段把 artifact 从工具记录完全归一化成独立数据库表。
- 不删除旧 Markdown 文件名图片匹配逻辑，只保留兼容读取。
