# Chat.tsx 拆分：交接计划（2026-09-19）

`src/chat/Chat.tsx` 是应用级 God Component。本轮已把 7 批内容抽成独立 hook / 纯函数 / 组件（4303 → 3126 行），下面是**剩余批次**的执行计划。执行者按批次做，每批独立验收、独立提交；完成后由发起人（本文作者）按第 6 节的清单复检。

## 0. 现状

### 本轮已完成（工作区未提交）

| 批 | 产出 | 从 Chat.tsx 移出的内容 |
|---|---|---|
| B1 | `composerPreferences.ts` / `skillSelection.ts` / `optimisticSidebar.ts` / `conversationFields.ts` / `idleTask.ts`（各带 test） | localStorage 偏好、附件→技能推断、侧栏乐观项、会话字段小工具、idle 调度 |
| B2 | `hooks/useRightDock.ts` + `dock/dockPreviewTarget.ts` | 右栏开/宽/tab/workdir/树展开/预览请求 |
| B3 | `hooks/useChatToolIndicator.ts` + `toolAvailability.ts::deriveToolStatusHint` | 工具目录 / MCP / 供应商能力 / 审批策略快照与刷新 |
| B4 | `hooks/useConversationContext.ts` | 上下文统计、手动压缩 / 清空、压缩中集合、边界高亮、`onChatContext` / `onChatCompaction` 监听 |
| B5 | `hooks/useConversationMetaMutations.ts`；`useComposerDraft` 新增稳定的 `setters` 包 | 模型 / 思考 / 联网 / 多答 / 知识库 / 附加目录 / 运行时（含外部模型/沙盒/预设、审批后落沙盒）/ Goal 四操作 |
| B6 | `hooks/useMessageActions.ts`（含纯函数 `noteTitleFromContent`） | 编辑 / 删除 / 回到这里 / 建分支 / 存笔记 / 多答组选中 |
| B7 | `PendingInteractionSlot.tsx` + `toolApproval.ts::buildToolApprovalActions` | 待答复槽（审批卡 / 会话授权 / 面板追问）JSX 与三形态按钮组 |

每个 hook 都有同名 `.test.tsx`，**除 B7**（见 B7b）。

### 续批已完成（2026-09-19 晚，工作区仍未提交）

| 批 | 产出 | 说明 |
|---|---|---|
| B7b | `toolApproval.test.ts` 增补 `buildToolApprovalActions`；`PendingInteractionSlot.test.tsx` | 三形态按钮、accept 后才 persist、空快照 `null`、userPrompt 同引用 |
| B8 | `hooks/useSidebarLayout.ts` + `chatWindowGeometry.ts` | 折叠 / 宽度 / CSS 变量 / 窗口 min-size |
| B9 | `hooks/useSettingsExit.ts`（已有）接到 Chat.tsx | 220ms 退场、排队导航、中心页回会话刷新 |
| B10 | `useChatRouting` 中心页 opener 接到 Chat.tsx；`extensionsNavItemForView` | 去掉 Chat 内重复 wrapper |
| B11 | `ConversationTitlebarControls.tsx` | 顶栏控件 `memo`，Chat 仍 `useMemo` 包一层 |
| B12 | `hooks/useAssistantActions.ts` | 助手 / 搭建创建与会话级切换；读会话用 ref |

B13 / B14 未做（可选）。`Chat.tsx` 现约 **2822** 行。验证：`tsc` / 定向 eslint / `vitest run src/chat` **190 files / 1517 tests** / `architecture:check` 全绿。

### 验证基线（全部通过）

```
npx tsc --noEmit
npx eslint <touched files> --max-warnings 0
npx vitest run src/chat          # 190 files / 1517 tests
npm run architecture:check       # 0 exceptions
```

### 提交注意

工作区里混着**与本拆分无关**的未提交改动：`SubAgentAvatar.tsx`、`emptyHero.*`、`kivioBlob*`、`streamPreviewOwner.*`、`streamingStore.render.test.tsx`、`src-tauri/**`、`docs/**`。拆分提交**只能包含** `src/chat/Chat.tsx` 与上表列出的新文件 / 改动文件（`useComposerDraft.ts`、`toolApproval.ts`、`toolAvailability.*`）。建议 `git add -p` 或按路径 add；一批一个 commit，形如 `refactor(chat): extract useConversationContext from Chat.tsx`。

## 1. 执行者必须遵守的约定

1. **纯搬迁，不改行为。** 错误文案、`console.error` 前缀、confirm 文案、超时毫秒数原样保留；不「顺手修」逻辑。发现疑似 bug 记在 PR 描述里，不动。
2. **读最新会话用 `currentConversationRef`，不要把 `currentConversation` 列进 handler 依赖。** 原因写在 Chat.tsx `currentConversationRef` 声明处的注释：handler 换身份会打穿 `MessageBubble` 的 memo（公式 remount 闪烁）。B5/B6 已按这个做，新 hook 同样。
3. **hook 返回值里的函数身份必须稳定**（`useCallback`，依赖齐全）。传给 hook 的对象参数若非稳定引用，先在源头 `useMemo`（参考 `useComposerDraft().setters`）。
4. **ESLint `react-hooks/exhaustive-deps` 是 error 级**（`--max-warnings 0`）。从 hook 返回的 setter 不被识别为稳定，需要列进依赖数组（见 Chat.tsx 里 `setContextState` / `resetContext` / `setContextLoading` 的处理）。
5. **文件归属：** hook 放 `src/chat/hooks/useXxx.ts`，纯函数放 `src/chat/xxx.ts`，组件放 `src/chat/Xxx.tsx`。非组件导出不要放进 `.tsx`（Fast Refresh，见 `settings/memoryLayers.ts` 注释）。DOM 相关测试文件后缀必须是 `.test.tsx`（vite.config 只对它启 jsdom）。
6. **测试写在 hook 的契约上，不是实现上：** 用 `renderHook` + mock `chatApi`/`api`，覆盖「无会话时只改草稿」「切走后迟到结果被丢」「失败路径报到哪个会话」这三类边界。参考 `useConversationMetaMutations.test.tsx`。
7. **改 Chat.tsx 不要用 PowerShell `Set-Content`**——会写入 BOM 触发 `no-irregular-whitespace`。用编辑器或 node 脚本（先 strip `\uFEFF`）。
8. **不碰以下区域**（它们是 R4a–R4e 已经收口的 owner 接线，不属于本次拆分）：`applyConversation` 家族、`freezeStreamSnapshot` / `settleStreamingPreview` / `restoreStreamingPreview`、`finishStreamingRun*`、`settlementPorts`、全部 `useTauriEvent(api.onChat*)` 流事件监听、`sendController` / `runCommands` / `queueCommands`、`navigation`。**尤其 `settleStreamingPreview` 的调用顺序（CLAUDE.md「settle 帧不闪契约」）不可动。**
9. 每批完成后跑第 0 节四条命令，全绿再提交。

## 2. 剩余批次

行号是 3126 行版本的近似位置，以**符号名**为准。

### B7b · 补 B7 的测试（优先，最小）

- 新增 `src/chat/toolApproval.test.ts`：`buildToolApprovalActions` 三形态（`exitplanmode` / `create_plan` → 计划批准；`enterplanmode` → 进入计划；其它 → 普通）。断言：按钮 label 顺序、最后一个 `primary + hint 'Ctrl+↵'`、`disabled` 跟 `submitting`；点计划档位按钮 → `resolve(true, false, mode)`，**resolve 返回 true 才**调 `persistSandbox(mode)`，返回 false 不调；`enterplanmode` 的「总是允许」→ `resolve(true, true)` 后 `persistSandbox('plan')`；拒绝 → `resolve(false)` 且不 persist。
- 新增 `src/chat/PendingInteractionSlot.test.tsx`：快照三项全空 → 渲染 `null`；只有 `sessionConsent` → 一张卡两个按钮，点击分别 `onResolveSessionConsent(false/true)`；`userPrompt` 相同 payload 重渲 → `AskUserBlock` 拿到**同一个** `toolCall` 对象引用（这是 `useMemo` 存在的理由，注释里写了）。`AskUserBlock` / `ApprovalCard` 可 `vi.mock` 成记录 props 的桩。

### B8 · `hooks/useSidebarLayout.ts`

移出：`sidebarCollapsed` / `sidebarWidth` state（~L368-369）、`setSidebarCollapsedPersisted`、`handleSidebarWidthChange`、`--chat-sidebar-width` 的 `useLayoutEffect`、`handleCollapseSidebar`、窗口 min-size `useEffect`（~L2404-2493）。

```ts
export function useSidebarLayout(): {
  collapsed: boolean
  width: number
  setCollapsed: (collapsed: boolean) => void   // = setSidebarCollapsedPersisted
  collapse: () => void                          // = handleCollapseSidebar
  setWidth: (width: number) => void             // = handleSidebarWidthChange
}
```

- 把 min-size 计算抽成纯函数 `chatWindowMinSize(collapsed, sidebarWidth)`（放同文件导出或 `chatWindowGeometry.ts`），单测两种分支。
- 依赖：`getRememberedChatSidebarCollapsed` / `rememberChatSidebarCollapsed` / `getRememberedSidebarWidth` / `rememberSidebarWidth` / `rememberChatSize` / `CHAT_MIN_SIZE_COLLAPSED` / `measureChatSurface` / `isTauriRuntime`——照 Chat.tsx 现有 import 搬。
- 测试：`localStorage` 持久化往返；`setCollapsed` 后 `.chat-window-shell` 的 CSS 变量；`isTauriRuntime()` 为 false 时不动窗口（mock `./utils`）；最大化 / 全屏时**不**调 `setMinSize`（mock `@tauri-apps/api/window` 的 `getCurrentWindow`）。
- Chat.tsx 里 `sidebarCollapsed` / `sidebarWidth` 的其余读取点（Sidebar props、titlebar、`inputBarProps` 等）改读 hook 返回值即可，不改语义。

### B9 · `hooks/useSettingsExit.ts`

移出：`settingsExiting` state、`handleSettingsClose`、`prevChatViewRef` + 「回到会话视图刷新技能与工具指示器」effect、`runAfterLeavingSettings`、`pendingAfterSettingsCloseRef`（~L1178-1239）。`settingsRef` 保留在页面（它是传给 `SettingsShell` 的 ref），作为参数传入。

```ts
useSettingsExit({
  chatView, setChatView,
  settingsRef,                     // RefObject<SettingsShellHandle>
  currentConversationIdRef,
  syncConversationRoute,           // 来自 useChatRouting
  onReturnedToConversation: () => { void loadSkills(); void refreshToolIndicator() },
}) => { settingsExiting, closeSettings, runAfterLeavingSettings }
```

- 220ms 的退场时长是与 CSS 对齐的常量，保留数值并加注释。
- 测试（fake timers）：`closeSettings` 先置 `settingsExiting=true`，220ms 后切回 `conversation` 并调用 `completeSettingsExit` 与 `onReturnedToConversation`；`runAfterLeavingSettings` 在非设置视图直接执行 action；在设置视图且 `settingsRef.current` 存在时只调 `requestClose()` 并把 action 挂起，随后 `closeSettings` 走完才执行；`settingsRef.current` 为 null 时立即退出并执行。从 `skill`/`mcp`/`assistants`/`knowledge`/`settings` 回到 `conversation` 触发 `onReturnedToConversation`，从 `notes` 回来不触发（保持现有白名单）。

### B10 · 中心页导航收口进 `useChatRouting`

`openEmbeddedSettings` / `handleOpenChatSettings` / `openAssistantCenter` / `openSkillCenter` / `openMcpCenter` / `openKnowledgeCenter` / `openNotesCenter` / `openAutomationsCenter` / `openExtensionsItem` / `extensionsActive`（~L1100-1176）全是「setChatView + syncXxxRoute」的一对一映射。

- 先读 `hooks/useChatRouting.ts` 现有接口，把这些 opener 加为它的返回值（它已经拥有各 `syncXxxRoute`），而不是再开一个 hook。`openEmbeddedSettings` 需要 `setSettingsInitialTab`，作为参数传入。
- `extensionsActive` 改成纯函数 `extensionsNavItemForView(chatView): ExtensionsNavItem | null` 放 `chatRoutes.ts`（那里已经是路由谓词的家），加单测。
- `openExtensionsItem` 里的 `setExtensionsNavItem(item)` 一并进 hook；`extensionsNavItem` state 只有这一处写入，读取点检查后一起搬。

### B11 · `ConversationTitlebarControls.tsx`

`conversationTitlebarControls` 这段 ~115 行 JSX（~L2672-2787）抽成 `memo` 组件，props 就是它现在闭包里读到的全部值：`activeAgentRuntime`、`currentConversation`（只用 `id` / `messages.length` / 是否 popout → 传 `conversationId` + `locked` 两个标量更利于 memo）、`usesExternalRuntime`、`activeProviderId` / `activeModel`、`handleRuntimeChange` / `handleExternalModelChange` / `handleModelChange` / `handleThinkingLevelChange` 等。Chat.tsx 保留 `useMemo(() => <ConversationTitlebarControls .../>, [...])` 以稳定元素身份（`pendingSlot` 同款）。

- 不需要行为测试；一个 render smoke test（`.test.tsx`，mock 掉 `RuntimePicker` / `ModelSelector` / `ExternalModelSelector` / `ThinkingLevelSelector`）确认外部运行时渲染 `ExternalModelSelector`、内置渲染 `ModelSelector` 即可。

### B12 · `hooks/useAssistantActions.ts`

移出：`handleStartAssistantChat` / `handleStartBuilderChat` / `handleApplyAssistant` / `handleSelectAssistant`（~L1668-1742）。

```ts
useAssistantActions({
  currentConversationRef, navigation, refreshSidebar, refreshContextStats,
  applyConversationIfCurrent, setStreamErrorForConversation,
  setAssistantStreamStatsByMessageId,
  identity: { activeProviderId, activeModel, projectId, projectName, setId },  // 页面 useMemo 后传入
}) => { startAssistantChat, startBuilderChat, applyAssistant, selectAssistant }
```

- `handleApplyAssistant` 现在依赖 `currentConversation`，改读 ref（约定 2）。
- `setStreamError`（模块级函数，写 `streamingStore` coarse）在 hook 里按 `useMessageActions.ts` 的做法本地定义。
- 测试：有会话 `selectAssistant(a)` → `updateConversation(id, { assistantId })` + `refreshContextStats`；无会话 → `createConversation(..., assistant.id, ...)` 并 `commitCreatedConversation`；`selectAssistant(null)` → `assistantId: ''`；创建失败且 lease 仍 current 才写 streamError；助手自带 provider/model 优先于 active。

### B13 · `hooks/useChatBootstrap.ts`（可选，收益中）

移出：`loadDefaultModel`、`skills` state + `skillProjectCwdRef` + `loadSkills`、首屏 `useEffect`（`loadDefaultModel` + idle `loadSkills`）、`subscribeSettings` → `setUiLang` effect、中心页 chunk 空闲预取 effect（~L1009-1098）。`uiLang` state 也一起进（它只在这两处写）。

- `loadDefaultModel` 的 provider 解析逻辑已在 `resolvePreferredChatModel` 里，hook 只负责编排；测试重点是「失败回落 dev-provider/dev-model」和「只有 last 与 preferred 一致且 settings 没存才 `persistLastChatModelToSettings`」四象限。
- 预取 effect 里的 `import('./AssistantCenter')` 等**保留静态字面量**（`architecture:check` 把字面量动态导入纳入图；改成变量会让门禁失明）。

### B14 · 侧栏 handler 包（可选，收益低，最后做）

`handleSidebar*` 十余个（~L2495-2650）多是 `runAfterLeavingSettings(() => ...)` 的薄包装。做法是 `useMemo` 成一个 `sidebarProps` 对象或抽 `useSidebarHandlers`。只有在 B9 落地后再做；若做完发现只是把 150 行原样挪到另一个文件而没有减少页面依赖，可以放弃这一批，写明原因即可。

### 明确不做

- 流事件监听 / send / run / settle 接线（约定 8）。
- `inputBarProps` / `messageListProps` 两个大 `useMemo`：它们就是页面「把领域动作接到展示」的本职，拆了只会把 prop 清单复制一遍。
- 行数不是验收指标（convergence spec R4e 原话）；目标是 Chat.tsx 剩下的每一段都能在 `docs/architecture-state-ownership.md` 里说出 owner。

## 3. 每批验收

1. `npx tsc --noEmit`、`npm run lint`、`npx vitest run src/chat`、`npm run architecture:check` 全绿。
2. `git diff src/chat/Chat.tsx` 里**只有删除与 hook 调用替换**，没有出现在新文件里不存在的新逻辑。
3. 新 hook 的每个导出函数至少一条测试；失败路径至少一条。
4. 手动冒烟（`npm run dev`）：B8 拖侧栏宽度 / 折叠后窗口 min-size 变化；B9 在设置页点侧栏会话能正常离开设置并跳转；B10 扩展 nav 高亮正确；B12 欢迎页选专家能开新对话。

## 4. 全部完成后

- `docs/prd/architecture-convergence-spec.md` 加一节 `### R4f Chat 页面组合层拆分`，格式同 R4e（旧入口 / 新 owner / 删除的重复规则 / 失败用例与复验），列出 B1–B14 的 hook 名与测试数量，并明确写「R4 仍未关闭」若还有 owner 未收归。
- `docs/architecture-state-ownership.md` 的 Chat 前端小节按新 hook 逐条补 owner（上下文用量 → `useConversationContext`，元数据写入 → `useConversationMetaMutations`，消息操作 → `useMessageActions`，待答复槽 → `PendingInteractionSlot`，侧栏布局 → `useSidebarLayout`，……）。
- `CLAUDE.md` 的 Frontend Submodules 段补一句 `src/chat/hooks/` 是页面级 owner hook 的家。

## 5. 提交切分

一批一个 commit，顺序 B7b → B8 → B9 → B10 → B11 → B12 → B13 → B14 → docs。每个 commit 只包含该批的新文件 + Chat.tsx + 直接被改的既有文件。**已完成的 B1–B7 由发起人自己提交**，执行者从干净的 Chat.tsx 3126 行版本开始。

## 6. 复检清单（发起人用）

- [ ] `git log --stat` 每个 commit 只碰该批文件；没有把第 0 节列出的无关改动带进来。
- [ ] 对每个新 hook：`git diff` 旧 Chat.tsx 段落 vs 新文件，逐行比对——文案、超时、confirm、`console.*` 前缀、`void`/`await` 一致；错误报到的 conversationId 来源一致（ref vs 参数）。
- [ ] 每个 handler 的依赖数组不含 `currentConversation` 对象；含 hook 返回的 setter 时已列入。
- [ ] 传给 hook 的对象参数在页面侧是稳定引用（`useMemo` / `useState` 初始化 / ref），否则 `useCallback` 形同虚设。
- [ ] 测试不是对着实现抄：至少一条「切走后迟到结果不落地」与一条失败路径。
- [ ] Chat.tsx 中被移出符号的 import 已清干净（`tsc` 的 TS6133/TS6196 为 0）。
- [ ] `PendingInteractionSlot` / `ConversationTitlebarControls` 这类元素在 Chat.tsx 仍被 `useMemo` 包住，否则接收它的 memo 子组件每帧重渲。
- [ ] `architecture:check` 无新增 temporary exception；预取 effect 的动态导入仍是字面量。
- [ ] B13 若做了：`uiLang` 只剩 hook 一个写入点。
- [ ] 文档三处（spec / state-ownership / CLAUDE.md）已更新，且没有把 R4 标成已完成。
