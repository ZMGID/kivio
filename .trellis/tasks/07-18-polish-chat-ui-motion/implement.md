# Implement — Chat 交互动效打磨

按价值/风险排序，每阶段可独立验证与回滚。低风险在前，最大工程（R3）在后。

## Phase 1 — 级联入场（R1，低风险，先做）
- [x] 1.1 会话列表 stagger：`ConversationList.tsx:117` map 加 `index`；`:158` 行容器加 `chat-motion-row` + `--chat-motion-delay`（`Math.min(index,12)*18`ms）；重命名分支不加。
- [x] 1.2 搜索结果 stagger：`Sidebar.tsx:342` map 加 index；`:348` 结果按钮同款。
- [~] 1.3 下拉列表项 stagger（**刻意延后**：高频下拉每次开都级联反拖慢手感，popover 入场已够）：先做 `ModelSelector.tsx`（favorite+grouped 跨段累加 index）+ `InputBar` slash 面板；其余选择器按同 pattern 增量。
- [~] 1.4 右键菜单项级联（**刻意跳过**：菜单 button/div 混排 + 子菜单常驻，nth 计数易错、性价比低）：`src/index.css` 加 `.chat-motion-popover [role="menuitem"]` nth-of-type 递增 delay；确认 `ConversationContextMenu` 分隔符/子菜单不打乱计数（必要时加 `data-menuitem`）。
- **验证**：`npm run typecheck` + `npm run lint`；手动：切库/搜索/开模型下拉/开右键菜单，观察逐项进入；开 reduced-motion 确认瞬现。

## Phase 2 — 入场一致性（R2，低风险）
- [x] 2.1 新增 `.chat-motion-bubble-in`（fade+translateY(6px)+scale(0.98)）于 `src/index.css`，补进 reduced-motion 具名兜底；`MessageBubble.tsx:922/1065` 把 `chat-motion-fade-up` 换成新变体（仅流式播放门控不变）。
- [x] 2.2 空态 hero 两级：`Chat.tsx:3624` 移除栈级 `chat-motion-fade-up`，标题挂（delay 0）、InputBar 包裹层挂（delay 120ms）。
- **验证**：typecheck+lint；手动新建对话看空态标题先入、气泡入场带轻微缩放。

## Phase 3 — error/cancel pop（R4，XS）
- [x] 3.1 `ToolCallBlock.tsx:1300-1351` 把 `justCompleted` → pop 门控扩展到 error/cancelled/skipped，仅动效不加色。
- **验证**：触发一次失败/取消的工具调用，确认实时 pop、历史不弹。

## Phase 4 — 按钮点击动画（R6，纯 CSS，用户重点，优先做）
- [x] 4.1 `src/index.css` `.kv-btn`（`:3074`）+ `.kv-icon-btn`（`:3031`）加 `:active:not(:disabled){transform:scale(0.97)}` + transition 补 `transform`。
- [x] 4.2 reduced-motion 块（`:1648`）加 `.kv-btn:active,.kv-icon-btn:active{transform:none}`。
- [x] 4.3 chat 内 `active:scale-90` 处（`InputBar.tsx:1796`、`MessageList.tsx:657`）尺度对齐（可保留，仅一致性检查）。
- **验证**：typecheck+lint；手动点击各处按钮看回弹；reduced-motion 下无缩放。

## Phase 5 — 视图/页面切换入场（R5，用户重点）
- [x] 5.1 会话切换：`Chat.tsx:3727` 容器 `chat-motion-fade` → `chat-motion-fade-up`（已 keyed，一行）。
- [ ] 5.2 侧边栏 scope 切换：会话列表 wrapper 确认/加 `key={scope}` + `chat-motion-fade-up`。
- [x] 5.3 chat 内子视图/分区切换（`#chat/settings` 子路由、项目/集视图）：wrapper 加 `key` + `chat-motion-fade-up`。
- [ ] 5.4（可选）方向性前进/后退：仅当有明确前进后退栈时加 `chat-motion-slide-in-fwd/back`（新 keyframe + 补兜底）。默认跳过。
- **验证**：切会话/切 scope/进出 chat 设置，新视图 fade-up 进入；reduced-motion 瞬现；确认 keyed remount 不误重置需保留的状态。

## Phase 6 — 退出动画（R3，最大工程，最后做）
- [x] 6.1 新增 `src/chat/useCloseAnimation.ts`（open→closing→unmount + onAnimationEnd + 超时兜底），仿 `ChatAttachments.tsx:115,189`。
- [x] 6.2 应用到模态：`ProjectDialog`/`SetDialog`/`SkillCenter`（内容 `chat-motion-exit`、backdrop 淡出）。
- [x] 6.3 应用到右键菜单：`ConversationContextMenu`/`ProjectContextMenu`/`SetContextMenu`/`ChatSectionMenu`（closing 挂 `chat-motion-exit`）。
- **验证**：反复开关模态/右键菜单确认有退出动画且关闭后 DOM 卸载；reduced-motion 下仍能立即关闭不卡。
- 下拉选择器退场 = 后续增量，不在本次。

## 全局验证（收尾）
- [ ] `npm run lint`（--max-warnings 0）+ `npm run typecheck` 全绿
- [ ] 相关 `.test.tsx` 若受影响保持绿（`npx vitest run src/chat/...`）
- [ ] 手动冒烟：切会话、开关所有改动浮层、新建对话空态、失败工具、reduced-motion 开关

## 风险 / 回滚点
- Phase 6 是唯一有状态机风险的部分：若浮层关不掉，回滚 6.x 单独提交即可（Phase 1-5 互不依赖）。
- Phase 1/4/5 复用现有原语，纯 CSS 或加 class/key，风险最低，可先合入。
- 建议按 Phase 分开提交，Phase 1-5 可独立合入，Phase 6 独立评估。
