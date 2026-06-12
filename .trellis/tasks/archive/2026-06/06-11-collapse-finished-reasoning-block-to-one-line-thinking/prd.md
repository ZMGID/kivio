# Collapse Finished Reasoning Block to One-Line "Thinking"

## Goal

聊天消息里的思考过程块，在生成完毕后默认折叠为单独一行 "Thinking"（仅标题 + 展开箭头，正文完全隐藏），点击可展开全文。标签文案从「思考过程」改为 "Thinking"。

## 背景

当前 `ReasoningBlock.tsx` 折叠态仍显示末尾 3 行预览，一次多步 agent 回复会出现多个灰色思考块，占据的视觉面积比正文还大；且截断提示「…」因 inline span + 块级 markdown 渲染会独占一行，观感差。

## Requirements

- 生成完毕（`streaming=false`）：默认折叠为一行，仅显示 "Thinking" + chevron，正文不可见（高度 0，带现有 max-height 过渡动画）。任意非空 reasoning 都可折叠/展开（不再有 3 行以内不折叠的豁免）。
- 生成中（`streaming=true`）：Thinking 正文区域使用固定高度、有边框的滚动框，顶部和底部有柔和渐隐，内容自动跟随到底部，避免随 token 长短撑高或缩短；shimmer 标签文案改为 "Thinking…"。
- Thinking 正文区域不渲染 Markdown / fenced code block；按纯文本展示模型思考内容，保留换行和原始字符。
- 流式结束时自动收起；若用户在流式期间手动展开过，则尊重用户选择不强制收起（沿用 `userExpandedRef` 现有语义）。
- 标签与 `aria-label` 全部由「思考过程」改为 "Thinking"。
- 折叠态不再渲染「…」截断提示（随预览一起消失）。
- 不动 settings i18n 中 Lens 的 `lensThought`（属于 Lens 功能，非聊天）。

## Acceptance Criteria

- [ ] 生成完毕的思考块只占一行 "Thinking"，点击展开显示全文，再点收起。
- [ ] 流式期间仍能看到固定高度滚动框里的思考内容，顶部/底部有渐隐，标签为 "Thinking…"。
- [ ] Thinking 块内的 Markdown/code fence 只作为普通文本显示，不出现代码块卡片。
- [ ] 流式结束自动收起为一行（未手动展开过的情况下）。
- [ ] `npm run typecheck` / `npm run lint` 通过。
- [ ] 手工冒烟：用支持 reasoning 的模型发一条消息观察流式→收起全过程。

## Out of Scope

- 工具调用行的 UI 优化（source 挤压、结果预览人话化）——另立任务。
- Lens 侧思考过程展示。
