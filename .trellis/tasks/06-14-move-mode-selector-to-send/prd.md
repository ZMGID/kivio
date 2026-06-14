# 把聊天模式选择器移到发送键旁边

## Goal

聊天输入框（`src/chat/InputBar.tsx`）当前把「模式选择器」（`⚡ Act ⌄` 计划/编排模式 pill）放在输入框**底部工具条**里，紧挨「进入项目工作」。用户希望把它移到**顶部输入行、发送键 ↑ 的左边**，让顶部行变成 `+ / 调节 / 输入框 / [⚡ Act ⌄] / ↑`，底部只留「进入项目工作」。

## What I already know（代码事实）

- 顶部输入行：`<div className="flex items-end gap-2">`（InputBar.tsx:1550）= `+`(Plus) / 调节(SlidersHorizontal) / `textarea` / 执行(可选) / [停止|发送 ArrowUp]（送出按钮 1623-1639）。
- 底部工具条：`{(projectEntryEnabled || modeEntryEnabled) && (<div className="...mt-2 flex items-center justify-start gap-1.5 px-3">...)}`（1642-1753）= 项目选择按钮（1644-1675）+ 模式选择块（1676-1751，含 pill 按钮 + 下拉菜单）。
- 模式下拉菜单当前 `absolute left-0`（向右展开），放置/方向由 `projectPanelPlacementClass`（inline→`top-full`向下；否则`bottom-full`向上）和 `projectPanelOrigin`（`top left`/`bottom left`）控制（1226-1229）。
- `layout==='inline'` = 首页「What's top of mind?」顶部输入框（截图场景）。

## Requirements

- 模式选择块整体（pill 按钮 + 下拉菜单）从底部工具条移入顶部输入行，置于停止/发送按钮**正左侧**；与发送键底部对齐（`mb-0.5 shrink-0`）。
- 下拉菜单改**右对齐**：`left-0` → `right-0`，原点改 `top right`/`bottom right`（避免贴右边时向右溢出屏幕）；上下展开方向仍按 `layout` 不变。
- 底部工具条移走模式块后只剩项目选择：外层条件 `(projectEntryEnabled || modeEntryEnabled)` 收成只看 `projectEntryEnabled`；不留空壳。
- 不改模式切换逻辑（toggleModeMenu / setAgentPlanMode / AGENT_MODE_OPTIONS / 状态），只动布局与下拉对齐。

## Acceptance Criteria

- [ ] 首页顶部输入行顺序为 `+ / 调节 / 输入框 / ⚡模式pill / ↑`，pill 紧邻发送键左侧、对齐良好。
- [ ] 点 pill 弹出的模式菜单**右对齐、不溢出屏幕右缘**，inline 场景向下展开、会话页向上展开。
- [ ] 底部工具条只剩「进入项目工作」；模式块不再出现在底部。
- [ ] `projectEntryEnabled` 关闭时底部不渲染空容器；`modeEntryEnabled` 关闭时顶部不渲染 pill。
- [ ] `npm run typecheck` / `npm run lint` 通过。

## Out of Scope

- 输入翻译 / lens / 其它窗口。
- 模式本身的行为、文案、图标、颜色。

## Technical Notes

- 纯 `src/chat/InputBar.tsx` 单文件布局调整。注意 pill 是 `h-[26px]`、发送键 `h-9`，在 `items-end` 行里底部对齐。
- 下拉菜单的 `relative` 包裹 div 随块一起移动，right-0 相对它定位即可。
