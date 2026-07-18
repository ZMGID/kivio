# 借鉴 LiveAgent 打磨 Chat UI 交互动效

## Goal

参考 `cankao/LiveAgent`（React 19 + Tailwind v4 + Base UI）的**交互动画**，
补齐 Kivio Chat（`src/chat/`）与之相比缺失的动效。**范围限定在交互动画本身**——
不改色板/字体/圆角/工具配色等视觉身份，不做色令牌重构。

## Background

### Kivio 动效底座已成熟（复用，勿重造）
`src/index.css` 已有完整原语：
- 缓动令牌 `--kv-ease-standard`(0.22,1,0.36,1) / `-firm` / `-spring`(0.34,1.56,0.64,1，与 LiveAgent logo pop 同曲线) / `-out`；时长阶梯 `--kv-dur-instant/fast/normal/slow` = 90/150/220/320ms（`src/index.css:81-90`）
- 入场/交互 class：`chat-motion-fade-up`(读 `--chat-motion-delay`/`--chat-motion-duration`) / `-popover`(读 `--chat-popover-origin`) / `-row`(读 `--chat-motion-delay`) / `-pop`(spring) / `-fade` / `-modal-in` / `-exit` / `-reveal` / `-soft-pulse`
- **stagger 机制已落地**：`Sidebar.tsx:1053-1061` / `1197-1203` 项目/集列表已用 `chat-motion-row` + `style={{'--chat-motion-delay': Math.min(index,12)*18+'ms'}}`（18ms/项，cap 12）——即 LiveAgent `sidebarMenuItemIn` 的等价物
- **退场范式已存在**：`ChatAttachments.tsx:115,189` 的 `removing` state + `chat-motion-exit` + `onAnimationEnd` 延迟卸载
- **reduced-motion 全局兜底完整**（`src/index.css:1648-1724`，通配 0.01ms + 具名 `animation:none`）——复用现有 class/Tailwind 工具类的新动效自动被覆盖；仅新增独立 `@keyframes` 时才需手动补进兜底块

### 审计结论（research/audit-A|B|C）
Kivio 与 LiveAgent 在动效原语上基本持平甚至更细。真正在「交互动画」范围内的缺口：
列表/弹层项无 stagger、空态无分层编排、气泡入场缺 scale、浮层普遍无退出动画。

## Requirements

按价值/风险分阶段（详见 `implement.md`）。全部复用现有原语，不新增视觉。

- **R1 级联入场**：会话列表(`ConversationList.tsx:117,158`)、搜索结果(`Sidebar.tsx:342,348`)、各下拉/弹层列表项（ModelSelector/MultiModelSelector/ThinkingLevelSelector/RuntimePicker/PermissionPicker/AssistantPicker/InputBar 各面板）逐项 stagger；右键菜单项(`ConversationContextMenu` 等)用纯 CSS `[role=menuitem]:nth-child` 级联。复用 `chat-motion-row`/`-fade-up` + `--chat-motion-delay`。
- **R2 入场一致性**：气泡入场 `chat-motion-fade-up` 增加 `scale(0.98)`（对齐 LiveAgent `chatBubbleIn`，`MessageBubble.tsx:922/1065`）；空态 hero 由单一整栈 fade-up 拆为「标题→输入框」两级 delay（`Chat.tsx:3624`）。
- **R3 退出动画**：新增复用 hook（open→closing→unmount + `onAnimationEnd`，仿 `ChatAttachments` removing 范式），先覆盖模态对话框(`ProjectDialog`/`SetDialog`/`SkillCenter`)与右键菜单(`ConversationContextMenu` 等)：内容 `chat-motion-exit` 反向、backdrop `chat-motion-fade` 反向。其余下拉退场排后续。
- **R4 error/cancel 出现动效**：error/cancelled 状态图标出现时用 `chat-motion-pop`（`ToolCallBlock.tsx:1322-1351`），仅加动效不加强调色。
- **R5 视图/页面切换入场动画**（用户重点）：照 LiveAgent「keyed 视图容器 + 一次性入场 class」做法——切换时容器 remount 并重播入场。LiveAgent 参考：`settings-section-enter`/`settings-tools-view-enter`（keyed by tab）/`chat-history-scope-enter`（keyed by scope）/`git-review-pane-enter-forward|back`（前进后退方向 translateX ±24px）。Kivio 落点：会话区（`Chat.tsx:3727` **已 keyed by conversation id**，现仅 `chat-motion-fade` → 升级为 `chat-motion-fade-up`）、侧边栏 scope 切换列表、chat 内 settings 子路由/分区、项目/集视图。复用 `chat-motion-fade-up`；方向性前进/后退为可选增强。
- **R6 按钮点击动画**（用户重点）：照 LiveAgent `active:scale-95`（或 `[0.98]`）+ `transition-transform duration-150 ease-out` + `motion-reduce:active:scale-100` 做法。Kivio 落点：集中加到基类 `.kv-btn` / `.kv-icon-btn`（`src/index.css:3074/3031`，现**无 `:active` 按压**）一处覆盖全站动作按钮；chat 内非 kv-btn 的图标/发送键已有 `active:scale-90`，统一到同一尺度。reduced-motion 下重置 `transform:none`。

## Acceptance Criteria

- [ ] 会话列表、搜索结果、至少一处下拉列表项刷新时逐项级联进入（18ms/项、cap 12），观感与项目/集列表一致
- [ ] 右键菜单打开时菜单项逐项进入
- [ ] 消息气泡入场为 fade + translateY + scale(0.98)；空态 hero 标题先于输入框进入
- [ ] 模态对话框与右键菜单**关闭时有退出动画**（不再瞬间消失），且关闭后 DOM 正确卸载
- [ ] error/cancelled 工具状态实时出现时有 pop 动效（历史批量渲染不弹）
- [ ] 切换会话 / 侧边栏 scope / chat 内子视图时，新视图有一次性入场动画（fade + 上浮），非瞬现
- [ ] 所有 `.kv-btn` / `.kv-icon-btn` 及 chat 动作按钮按下时有轻微缩放反馈（scale ~0.97），松开回弹
- [ ] `prefers-reduced-motion: reduce` 下以上动效全部退化为瞬时/静态（含按钮 `transform:none`、视图入场瞬现）
- [ ] `npm run lint` + `npm run typecheck` 通过；手动冒烟：切会话/开关菜单与模态/新建对话空态

## Out of Scope（本次不做）

- 工具按类型四色强调（bash绿/file蓝/…）—— 与 Kivio 刻意纯灰阶身份冲突（`ToolCallBlock.tsx:381-383`）
- 色令牌重构（terracotta/紫等硬编码收敛为 `--kv-chat-*`）—— 属颜色组织，非交互动画
- jump-to-bottom 磨砂加强、error 强调色 —— 视觉项
- hero logo pop/float/halo、建议卡级联 —— Kivio 无 logo/建议卡，属功能缺口
- 全部 ~15 个下拉的退出动画全量铺开 —— 成本高、杠杆低，列为后续（退出动画本轮只做模态+右键菜单）
- 非 Chat surface（translator/lens）；chat 内 settings 子视图纳入 R5，独立 settings 窗口不改

## Notes

- 退出动画（R3）是与 LiveAgent 最大的结构差距（LiveAgent 靠 Base UI `data-ending-style` 免费拿到 enter/exit；Kivio 手写浮层只做 enter）。本次只做高价值子集 + 建复用 hook，为后续全量铺路。
- 实施前需 grep 确认 `subagent-sparkle` / 紫 `chat-motion-subagent-shimmer`（`src/index.css:1380,1588`）是否死代码（research B6）——本次不依赖它，仅避免误增重复。
