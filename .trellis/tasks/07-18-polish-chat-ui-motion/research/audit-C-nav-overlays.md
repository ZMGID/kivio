# Research: Chat UI Motion Audit — Group C (Navigation / Overlays / Global)

- **Query**: 盘点 Kivio Chat 侧边栏/导航/浮层 surface 的入场与交互动效现状，对照 LiveAgent 找缺口（只读审计）
- **Scope**: internal (Kivio `src/chat/` + `src/index.css`) + reference (LiveAgent `agent-gui/src/index.css`)
- **Date**: 2026-07-18

## 0. Kivio 动效原语现状（`src/index.css`）

复用基座，实现完整，**这些是本任务的乐高积木**：

| 原语 | 定义 file:line | 语义 | reduced-motion 兜底 |
|---|---|---|---|
| `--kv-ease-standard` `.firm` `.spring` `.out` | `src/index.css:81-84` | 四条曲线：入场主曲线 / 拖拽 / 弹 / 收尾 | — |
| `--kv-dur-instant/fast/normal/slow` | `src/index.css:87-90` | 90/150/220/320ms | — |
| `.chat-motion-popover` (+`@keyframes`) | `src/index.css:1057-1060`, `1325-1334` | popover/dropdown/菜单入场：`translateY(-3px) scale(.985)`→归位，`--chat-popover-origin` 控 transform-origin | 具名 `animation:none`（`1670`） |
| `.chat-motion-row` (+`@keyframes`) | `src/index.css:1062-1065`, `1336-1345` | 列表项级联：`translateX(-4px)` 淡入，`--chat-motion-delay` 控延迟 | 具名 `animation:none`（`1671`） |
| `.chat-motion-modal-in` | `src/index.css:1234-1247` | 居中模态入场：`translateY(6px) scale(.97)`→归位 | 仅全局 0.01ms（**故意**，保 animationend，`1651`） |
| `.chat-motion-fade` | `src/index.css:1220-1231` | 纯透明度淡入（模态 backdrop 等） | 同上（故意，`1651`） |
| `.chat-motion-exit` | `src/index.css:1203-1217` | 退出：`scale(.85)` 淡出，`both`+`pointer-events:none` | 同上（故意，保 animationend 清 DOM，`1651`） |
| `.chat-motion-pop` | `src/index.css:1250-1266` | 微反馈弹入（✓ 复制等），spring 曲线 | 同上（故意，`1651`） |
| `.chat-motion-fade-up` | `src/index.css:1052-1055`, `1301-1310` | `translateY(8px)` 淡入，`--chat-motion-delay`/`--chat-motion-duration` 可调 | 具名 `animation:none`（`1669`） |
| `.chat-motion-reveal` `.chat-motion-reasoning-body` | `src/index.css:1091-1129` | 高度展开（grid 0fr→1fr） | 具名处理（`1682-1714`） |

**全局兜底**：`@media (prefers-reduced-motion: reduce)`（`src/index.css:1648-1724`）用通配符把所有 `animation-duration`/`transition-duration` 压到 `0.01ms`（非 0，为保 `animationend`/`transitionend` 事件），再对具名类补 `animation:none`。**结论：任何新增动效只要复用上述原语，或使用 Tailwind `transition-*`/`animate-*` 工具类，都会被全局兜底自动覆盖 —— 无需额外为 reduced-motion 写代码。** 唯一例外是若新增独立 `@keyframes` 且逻辑依赖 `animationend`，需按现有注释（`1651`）刻意留在全局兜底而不进具名 `animation:none` 列表。

---

## 1. 逐 surface 现状

### A. 会话列表项入场（ConversationList）— **缺 stagger（本组最高价值缺口）**
- 组件：`src/chat/ConversationList.tsx`；行渲染 `conversations.map((conv) => {...})` **无 index**（`ConversationList.tsx:117`），行容器 `<div>` 在 `ConversationList.tsx:158-166` 只有 `group relative flex ... rounded-lg`，**无 `chat-motion-row`、无 delay、无任何入场动画**。
- 该组件被 Sidebar 复用 3 处（`Sidebar.tsx:1129`、`1270`、`1332`），改一处覆盖全部会话列表。
- 现状：会话切库/刷新/搜索时列表整块瞬现，无级联。

### B. 项目 / 集 侧栏列表项入场 — **已有 stagger（等价物已存在，勿重造）**
- 项目：`src/chat/Sidebar.tsx:1053-1061` 行容器已用 `chat-motion-row`，`style={{ '--chat-motion-delay': `${Math.min(index, 12) * 18}ms` }}`（cap 12 项、18ms/项）。
- 集：`src/chat/Sidebar.tsx:1197-1203` 同款。
- **这正是 LiveAgent `sidebarMenuItemIn`（~20ms/项 递增）的 Kivio 对应实现，已落地。** 会话列表（surface A）只是漏了同一套 pattern。

### C. 搜索结果列表项入场 — 缺 stagger
- `src/chat/Sidebar.tsx:342-380`（搜索覆盖层 `SearchOverlay` 内）`results.map((conversation) => {...})` **无 index、无 `chat-motion-row`**，结果按钮 `Sidebar.tsx:348-357` 仅 hover 过渡。整块瞬现。

### D. 会话右键菜单 — 有入场、无退出、菜单项无级联
- `src/chat/ConversationContextMenu.tsx:62-68`：根 `<div>` 用 `chat-motion-popover`（入场 OK）。
- **无退出动画**：`onClose`（`ConversationContextMenu.tsx:49/52`）直接触发父级卸载，portal 立即消失，无 `data-ending-style`/`isClosing`/`onAnimationEnd` 延迟卸载。
- 菜单项（`role="menuitem"` 按钮，`ConversationContextMenu.tsx:69-222`）**无逐项级联**（对比 LiveAgent `sidebar-context-menu [role=menuitem]:nth-child` 纯 CSS stagger，LiveAgent `index.css:1285-1300`）。
- 子菜单（添加到项目/集）用 `opacity + transition-opacity` hover 显隐（`ConversationContextMenu.tsx:93`、`151`），无位移。

### E. 项目 / 集 / 分区 右键菜单 — 同 D（入场 OK，无退出/无级联）
- `src/chat/ProjectContextMenu.tsx:45`、`src/chat/SetContextMenu.tsx:36`、`src/chat/ChatSectionMenu.tsx:50` 均 `chat-motion-popover`，同样无退出、无菜单项级联。

### F. 各类 popover / dropdown — 入场统一，无退出
`chat-motion-popover` 已覆盖全部浮层（入场一致），但**全部即刻卸载、无退出动画**：
- ModelSelector `src/chat/ModelSelector.tsx:167`、MultiModelSelector `MultiModelSelector.tsx:112-113`、ThinkingLevelSelector `ThinkingLevelSelector.tsx:101`、PermissionPicker `PermissionPicker.tsx:122`、RuntimePicker `RuntimePicker.tsx:116/269`、AssistantPicker `AssistantPicker.tsx:87-88`、SourcesButton `SourcesButton.tsx:151-152`、ContextIndicator `ContextIndicator.tsx:223-224`、AgentTodoIndicator `AgentTodoIndicator.tsx:105`、BackgroundJobsIndicator `BackgroundJobsIndicator.tsx:115`、InputBar 多个面板（`InputBar.tsx:1328/1379/1447/1735`，均含 `--chat-popover-origin`）。
- 这些多用条件渲染 `{open && <Popover/>}`，无 exit 状态机。

### G. 模态 / 对话框 — 入场完整（backdrop fade + 内容 modal-in），无退出
- ProjectDialog：backdrop `ProjectDialog.tsx:68` `chat-motion-fade`，内容 `ProjectDialog.tsx:75` `chat-motion-modal-in`。
- SetDialog：backdrop `SetDialog.tsx:65`，内容 `SetDialog.tsx:72`，同款。
- SkillCenter 模态：backdrop `SkillCenter.tsx:671`，内容 `SkillCenter.tsx:679`，同款。
- **无退出动画**：`onClose` 直接卸载。backdrop `onMouseDown` 点击外部即关（`ProjectDialog.tsx:70`）。

### H. 会话 / 路由切换过渡 — 仅列表容器 fade，无内容 crossfade
- MessageList 外层容器 `src/chat/MessageList.tsx:635` 带 `chat-motion-fade`（容器 remount 时淡入）；空态 hero `Chat.tsx:3624` `chat-motion-fade-up`。
- **无 View Transition / crossfade / 前进后退方向过渡**（对比 LiveAgent `gitReviewPaneForward/Back`，LiveAgent `index.css:643-668`）。切会话时消息区靠 MessageList 容器 fade + 内部气泡各自 `chat-motion-fade`（`MessageBubble.tsx` 多处）呈现，无统一转场。

### I. 全局 reduced-motion — 覆盖充分
见 §0。通配兜底 + 具名规则已覆盖现有全部原语；新增动效若复用原语/Tailwind 工具类即自动覆盖。**唯一需注意**：新增独立 `@keyframes` 时决定是否进 `1669-1680` 的具名 `animation:none` 列表（依赖 animationend 的逻辑则刻意不进，见 `1651` 注释）。

---

## 2. 差距清单（每条：涉及文件 + 可复用原语 + 预估改动量）

| # | 差距 | 涉及文件（file:line） | 可复用 Kivio 原语 | 预估改动量 |
|---|---|---|---|---|
| C-1 ★ | **会话列表项无 stagger 级联**（本组最高价值） | `src/chat/ConversationList.tsx:117`（map 加 index）+ `158-166`（行容器加 class/style） | `chat-motion-row` + `--chat-motion-delay`（已被 B 验证的 pattern） | **小**。JSX 级：`.map((conv, index) =>`，行 div 加 `chat-motion-row` + `style={{'--chat-motion-delay': `${Math.min(index,12)*18}ms`}}`。**非纯 CSS**（当前 map 无 index，需加参）。一处改动覆盖 Sidebar 3 处用法。renaming 分支（`ConversationList.tsx:131-155`）建议不加动画。 |
| C-2 | **搜索结果无 stagger** | `src/chat/Sidebar.tsx:342`（map 加 index）+ `348-357`（按钮加 class/style） | 同 C-1（`chat-motion-row`） | **小**。同 C-1，但按钮是 `<button>` 直接渲染，加 index 与 class/style 即可。 |
| C-3 | **右键菜单项无逐项级联** | `ConversationContextMenu.tsx:69-222`、`ProjectContextMenu.tsx`、`SetContextMenu.tsx`、`ChatSectionMenu.tsx` | 可**纯 CSS**（对齐 LiveAgent `nth-child` 做法）：在 `.chat-motion-popover` 内对 `[role="menuitem"]` 加 nth-child animation-delay；或复用 `chat-motion-row` | **中**。纯 CSS 方案（新增一条 `src/index.css` 规则，`role=menuitem` nth-child 延迟）可一次覆盖所有菜单，无需动 JSX。注意 ConversationContextMenu 有分隔 `<div>` 与子菜单 wrapper，nth-child 计数需谨慎。 |
| C-4 | **所有 popover/菜单/模态无退出动画**（LiveAgent 用 Base UI `data-starting/ending-style` 延迟卸载） | §1-D/E/F/G 全部浮层组件 + 已有 `chat-motion-exit`/`chat-motion-fade`（backdrop） | `chat-motion-exit`（内容）+ `chat-motion-fade` 反向（backdrop）；或引入 `isClosing` 状态 + `onAnimationEnd` 延迟卸载（参考 `ChatAttachments.tsx:115/189` 的 `removing` 模式） | **大**。Kivio 无 Base UI，需给每个浮层加 exit 状态机（open→closing→unmount）。**逐组件改**，无一处覆盖全部的杠杆点。`ChatAttachments.tsx` 已有可复制的 `removing` + `chat-motion-exit` + animationend 卸载范式（`src/chat/ChatAttachments.tsx:115,189`）。建议按价值排序，先做模态/右键菜单。 |
| C-5 | **会话/路由切换无统一转场**（LiveAgent 有 `gitReviewPaneForward/Back`） | `src/chat/MessageList.tsx:635`（现仅容器 fade）、`Chat.tsx` 切会话逻辑 | 现有容器 `chat-motion-fade` 已提供基础；如需方向性可新增 keyframe | **中-大**。View Transition API 或方向 keyframe 需新增原语 + 触发逻辑。**优先级最低**（现有 fade 已可用，且 LiveAgent 的方向转场是 git-review 专属场景，Kivio 无对应双向导航需求）。 |

★ = 本组最高价值。

---

## 3. 关键判断

- **侧边栏 stagger 不是"从零补"，而是"补齐一处遗漏"**：项目/集列表（`Sidebar.tsx:1053-1061`、`1197-1203`）已有完整 `chat-motion-row` + delay pattern，**会话列表（C-1）和搜索结果（C-2）只是漏用同一套**。改法直接拷贝已有 pattern，风险与改动量都很低（JSX 加 index + 两个属性）。
- **stagger 加法：JSX 传 index（非纯 CSS）** —— 因 delay 用 inline `--chat-motion-delay` CSS 变量驱动，而当前两处 map 回调都没接 index 参数。若想纯 CSS，可改用 `nth-child(n) { animation-delay }`（如右键菜单 C-3 可行），但会话列表项数不定、cap 逻辑（`Math.min(index,12)`）用 CSS 表达繁琐，沿用现有 inline 变量方案更一致。
- **退出动画（C-4）是与 LiveAgent 最大的结构差距**：LiveAgent 靠 Base UI 生命周期免费拿到 enter/exit；Kivio 手写浮层、只做 enter。补 exit 需逐组件状态机，成本高，建议后续单独排期而非本轮全量。
- **reduced-motion 无需额外工作**：全局兜底（`src/index.css:1648-1724`）已覆盖复用原语的一切新动效。

## 4. Caveats / 未覆盖

- 未逐一展开 ProjectContextMenu / SetContextMenu / ChatSectionMenu 的完整 JSX（结构与 ConversationContextMenu 同构，均 `chat-motion-popover` 入场、无退出、无菜单项级联，已抽样确认根节点 class）。
- 未审计 Group A/B 归属的 surface（消息气泡、工具卡、reasoning、hero、compaction）——那些 `chat-motion-fade`/`fade-up`/`reveal`/`pop` 用法在 grep 结果中可见但属其他组。
- LiveAgent 侧仅对照 `crates/agent-gui/src/index.css`（CSS 层）；其 Base UI 组件 JSX 未读，`data-starting/ending-style` 的 React 接线细节以 CSS 注释（LiveAgent `index.css:1240-1243`）为据。
- "路由切换"在 Kivio 是同窗口 hash 子路由（`#chat/settings` 等）+ 组件条件渲染，非独立页面导航；未发现任何 route-level 转场原语。
