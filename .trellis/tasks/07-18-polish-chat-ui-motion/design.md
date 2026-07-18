# Design — Chat 交互动效打磨

## 原则
- 全部复用 `src/index.css` 现有原语，不新增视觉（颜色/字体/圆角不动）。
- stagger 沿用现有 inline `--chat-motion-delay` 方案（与 `Sidebar.tsx:1053` 一致），列表项数不定、有 `Math.min(index,12)` cap，inline 变量比 `nth-child` 表达更贴合；仅右键菜单（项数固定、无 index 参）用纯 CSS `nth-child`。
- 新增 keyframe 尽量避免；若必须，同步补进 reduced-motion 兜底块（`src/index.css:1648-1724`）。

## R1 级联入场

### 列表（会话/搜索）
`.map` 加 `index` 参，行容器加 `chat-motion-row` + `style={{ '--chat-motion-delay': \`${Math.min(index,12)*18}ms\` }}`。
- `ConversationList.tsx:117` map 加 index、`:158` 行容器加 class/style（重命名分支 `:131-155` 不加）。一处改动覆盖 Sidebar 3 处调用。
- `Sidebar.tsx:342` 搜索 map 加 index、`:348` 结果按钮加 class/style。

### 下拉/弹层列表项
同上 pattern，逐个下拉的 `renderRow`/`.map` 加 index + class/style。
注意 `ModelSelector` 有 favorite/grouped 两段渲染，index 需**跨段累加**（用单调计数器，非各段自 0）。
覆盖：`ModelSelector.tsx:174/184`、`MultiModelSelector`、`ThinkingLevelSelector`、`RuntimePicker`、`PermissionPicker`、`AssistantPicker.tsx:107`、`InputBar.tsx` slash(1391)/项目(1472)/模式(1740)。
> 若时间紧，此项可只做最高频的 ModelSelector + slash，其余同 pattern 增量补。

### 右键菜单项（纯 CSS）
在 `src/index.css` 新增一条规则：`.chat-motion-popover [role="menuitem"] { animation: chat-motion-row ...; }` 配 `:nth-child(n)` 递增 delay。
风险：`ConversationContextMenu` 内有分隔 `<div>` 与子菜单 wrapper，`nth-child` 计数会把它们算进去 → 改用 `:nth-of-type` 或给菜单项加统一 `data-menuitem` 属性用 `nth-of-type`，避免错位。

## R2 入场一致性

### 气泡 scale(0.98)
`chat-motion-fade-up` 的 `@keyframes`（`src/index.css:1301-1310`）当前只有 `translateY(8px)`。
**不直接改** `chat-motion-fade-up`（它被附件 chip/提示行等多处复用，加 scale 可能不合适）。
新增变体 `.chat-motion-bubble-in`（fade + translateY(6px) + scale(0.98)，用 `--kv-ease-standard`），只挂到气泡（`MessageBubble.tsx:922/1065`）。补 reduced-motion 兜底具名列表。

### 空态 hero 两级
`Chat.tsx:3624` 现整栈单一 `chat-motion-fade-up`。改为：移除栈级 class，标题(`:3625`)挂 `chat-motion-fade-up`（delay 0），InputBar 包裹层挂 `chat-motion-fade-up` + `style={{'--chat-motion-delay':'120ms'}}`。纯复用，无新 keyframe。

## R3 退出动画（最大结构差距，做高价值子集）

### 复用 hook（合理的抽象：~15 潜在调用点）
新增 `src/chat/useExitAnimation.ts`：
```
useExitAnimation(open: boolean): { mounted: boolean, closing: boolean, onAnimationEnd: () => void }
```
- open true → mounted true, closing false
- open 由 true→false → 保持 mounted、closing true；`onAnimationEnd` 触发后 mounted false
- reduced-motion 下动画 0.01ms 仍触发 animationend，卸载不卡（与 `ChatAttachments` 现范式一致，`src/index.css:1651` 注释保证）
参考实现范式：`ChatAttachments.tsx:115,189` 的 `removing` + `onAnimationEnd`。

### 应用（本次范围）
- 模态：`ProjectDialog`/`SetDialog`/`SkillCenter` —— backdrop 关闭时反向 `chat-motion-fade`（需 fade-out）、内容 `chat-motion-exit`。因现有 `chat-motion-fade`/`-modal-in` 只有入场，退场用已有 `chat-motion-exit`（内容 scale(0.85) 淡出）+ 新增/复用 backdrop 淡出（可用 `chat-motion-exit` 的 opacity 部分，或给 backdrop 加 `closing` 时 `opacity:0` + transition）。
- 右键菜单：`ConversationContextMenu`/`ProjectContextMenu`/`SetContextMenu`/`ChatSectionMenu` —— closing 时挂 `chat-motion-exit`。

> 下拉选择器（ModelSelector 等 ~11 个）的退场**排后续**，避免一次性 15 个状态机。hook 已就绪，后续增量接入。

## R4 error/cancel pop
`ToolCallBlock.tsx` `StatusIcon`（`:1300-1332`）现仅 success `justCompleted` 时 `chat-motion-pop`。把同一 `justCompleted` 门控扩展到 error/cancelled/skipped 分支（`:1322-1351`），出现时 pop。不加颜色。

## R5 视图/页面切换入场（照 LiveAgent「keyed 容器 + 入场 class」）

LiveAgent 的做法不是路由级 crossfade，而是：**给切换的视图容器加 React `key`（值变→remount）+ 一次性入场 class**。切 tab/scope/section 时新内容 fade+rise 进入。参考：`settingsToolsViewIn`(translateY 6px)、`chatHistoryScopeIn`(6px)、`settingsSectionIn`(14px+scale.985)、`gitReviewPaneForward/Back`(translateX ±24px 方向性)。全用 `cubic-bezier(0.16,1,0.3,1)`。

Kivio 落点（复用 `chat-motion-fade-up`，无需新 keyframe）：
- **会话切换**：`Chat.tsx:3727` 容器**已 `key={currentConversation?.id}`**，现挂 `chat-motion-fade` → 换成 `chat-motion-fade-up`（fade+上浮），一行改动。
- **侧边栏 scope 切换**：给按 scope keyed 的会话列表 wrapper 加 `chat-motion-fade-up`（对应 LiveAgent `chat-history-scope-enter`）。需确认/添加 `key={scope}`。
- **chat 内子视图/分区切换**（如 `#chat/settings` 子路由、项目/集视图切换）：视图 wrapper 加 `key` + `chat-motion-fade-up`。
- **方向性前进/后退（可选）**：若某处有明确前进/后退语义（如设置分区栈），可新增 `chat-motion-slide-in-fwd/back`（translateX ±20px）两个 keyframe（并补 reduced-motion 兜底）。非必须，默认用统一 fade-up。

风险：keyed remount 会重置该子树内部状态（滚动位置等）——会话区已 keyed（现状即如此，无回归）；新增 key 的 wrapper 需确认不误伤已保存状态（scope 列表本就随 scope 重取，安全）。

## R6 按钮点击动画（照 LiveAgent `active:scale`）

LiveAgent：交互元素挂 `active:scale-95`/`[0.98]` + `transition-[...,transform] duration-150 ease-out` + `motion-reduce:active:scale-100 motion-reduce:transition-none`。

Kivio 落点（集中，一处覆盖全站）：
- `.kv-btn`（`src/index.css:3074`）、`.kv-icon-btn`（`:3031`）现只有 hover，**加 `:active:not(:disabled) { transform: scale(0.97); }`** + 在 `transition` 里补 `transform var(--kv-dur-instant) var(--kv-ease-out)`。
- chat 内已用 `active:scale-90` 的发送键/jump-to-bottom（`InputBar.tsx:1796`、`MessageList.tsx:657`）统一到 `scale-0.97~0.95` 同一尺度（可保留现值，仅确保一致）。
- **reduced-motion 兜底**：在 `src/index.css:1648` 块内加 `.kv-btn:active, .kv-icon-btn:active { transform: none; }`（全局 0.01ms 只压时长、不移除 transform，须显式重置）。
- 尺度取值：press 用 `scale(0.97)`（比 0.90 克制，符合 Kivio 密度）；lucide 图标按钮可 0.95。

改动量：`.kv-btn`/`.kv-icon-btn` 各加一条 `:active` + transition 补 transform + 一条 reduced-motion 重置。**纯 CSS，零 tsx**，一次覆盖全站按钮。

## 数据流 / 契约
纯前端 UI 动效，无 Rust/IPC 改动，无持久化契约变化。全部改在 `src/chat/**` + `src/index.css`。

## 兼容 / 回滚
- 每项独立、可单独回滚（改 class/style 为主）。
- 风险点：R3 退出动画状态机若 `onAnimationEnd` 未触发会导致浮层关不掉 → hook 内加 `open` 变化的兜底（reduced-motion 下 0.01ms 仍触发；另可设超时保险）。
- R1 ModelSelector 跨段 index 若算错只是级联顺序不自然，不影响功能。

## 权衡
- 不做色令牌重构：用户明确「主要交互动画」，且令牌化是纯组织收益、无观感变化，性价比这次不占优，留作独立 chore。
- 退出动画只做子集：ponytail——先建 hook + 覆盖高可见浮层，不为对齐而一次性改 15 处。
