# Research: Audit B — 消息流 / 工具卡 / 推理块 动效现状（Kivio vs LiveAgent）

- **Query**: 盘点 Kivio Chat 消息流内容 surface（气泡 / 工具卡 / 推理块 / 子代理卡 / compaction / jump-to-bottom）的入场与状态动效现状，对照 LiveAgent 找缺口，重点评估领域色令牌 + 每类工具强调色
- **Scope**: internal（Kivio `src/chat/` + `src/index.css`）+ 参考对照（LiveAgent index.css / assistantBubbleUtils.ts）
- **Date**: 2026-07-18
- **约束**: 只读只记录；保留 Kivio 视觉身份（主色板/字体/圆角不动），只补交互质感 + 令牌组织

---

## 0. Kivio 现有可复用原语（`src/index.css`）

单一动效来源已建立（`src/index.css:75-91`）：

| Token | 值 | 用途 |
|---|---|---|
| `--kv-ease-standard` | `cubic-bezier(0.22,1,0.36,1)` | 入场/展开/弹出主曲线（≈ LiveAgent 的 `0.16,1,0.3,1`，几乎同族） |
| `--kv-ease-firm` | `cubic-bezier(0.2,0,0,1)` | 拖拽/重排 |
| `--kv-ease-spring` | `cubic-bezier(0.34,1.56,0.64,1)` | 轻弹回弹 |
| `--kv-ease-out` | `cubic-bezier(0.33,1,0.68,1)` | 颜色/透明度收尾 |
| `--kv-dur-instant/fast/normal/slow` | `90/150/220/320ms` | 时长阶梯 |

已有可复用 class（`src/index.css:1052-1266`）：
- `.chat-motion-fade-up` — opacity 0→1 + translateY(8px)（`@keyframes` 见 `1301-1310`），支持 `--chat-motion-delay` / `--chat-motion-duration`。
- `.chat-motion-reveal` / `.chat-motion-reveal.is-open` — grid-rows 0fr→1fr 高度展开（折叠区通用，`1091-1111`）。
- `.chat-motion-reasoning-body` — max-height 过渡（`1113-1129`）。
- `.chat-motion-pop` — spring scale 弹入（`1264`）。
- `.chat-motion-popover` / `-modal-in` / `-exit` / `-fade` / `-row` / `-soft-pulse`。
- shimmer 家族：`reasoning-shimmer-text`（`218`）、`chat-motion-subagent-shimmer`（紫，`1380`）、`chat-motion-tool-shimmer`（terracotta，`1406`）、`kv-skeleton`（`1278`）。
- compaction 专属：`chat-compaction-divider(--animate/--pre-enter)`、`chat-compaction-progress`、`chat-compaction-summary-*`（`1430-1585`）。
- subagent 专属：`subagent-sparkle(.is-running)` 字符摩点循环（`1588-1636`）、`kv-card-eyebrow--running` 光标闪烁（`1640-1646`）。
- **`prefers-reduced-motion` 全局兜底已完整**（`1648-1705`）：`*` 压到 0.01ms + 具名 `animation:none` 列表 + shimmer/skeleton/sparkle 各自静态兜底。**任何新增动效必须挂进这套兜底。**

**结论：Kivio 的动效原语层已相当成熟，缺口不在"缺原语"，而在"缺领域色令牌"和"部分 surface 未接入现有原语"。**

---

## 1. 逐 surface 现状

### 1.1 消息气泡入场（user / assistant）
- **组件**: `src/chat/MessageBubble.tsx`
- **现状**: 已有统一入场，但**仅流式消息播放**。`playEntranceAnimation = messageStreaming`（`MessageBubble.tsx:850`），user 气泡 `MessageBubble.tsx:922`、assistant 气泡 `MessageBubble.tsx:1065` 条件挂 `chat-motion-fade-up`。
- **刻意设计**（`MessageBubble.tsx:848-849` 注释）：历史消息被虚拟列表反复卸载/挂载，若都播入场会在滚动时"整屏刷新感"，故历史气泡不动。
- **对照 LiveAgent**: `.chat-bubble-enter` = `chatBubbleIn 0.3s`，opacity 0→1 + **translateY(6px) + scale(0.98)**（`LiveAgent index.css:947-972`）。Kivio 的 `chat-motion-fade-up` 只有 translateY(8px)，**无 scale(0.98)**。
- **差异**: 观感差异极小（都是 fade-up）；Kivio 缺 scale 微缩。

### 1.2 工具调用块 ToolCallBlock（内联行式）
- **组件**: `src/chat/ToolCallBlock.tsx`，普通工具走 `DefaultToolCallBlock`（`ToolCallBlock.tsx:1380-1477`）。
- **形态**: **内联灰字行**，非卡片 —— 容器 `not-prose mb-1 text-[12.5px] text-neutral-500`（`ToolCallBlock.tsx:1405`），**无边框、无背景、无磨砂**。
- **图标**: `ToolTypeIcon`（`ToolCallBlock.tsx:1334-1378`）按 `toolGlyph()`（`107-166+`）映射工具→lucide 图标（read/write/edit/bash/grep/glob/ls/… 覆盖齐全）。**图标颜色一律 `text-neutral-400/500`，无按类型着色。**
- **状态动效**:
  - running: 工具名挂 `chat-motion-tool-shimmer`（terracotta 流光，`ToolCallBlock.tsx:1421`）+ 图标 `animate-pulse`（`1356`）。
  - success/completed: ✓ 用单一 terracotta `text-[#C56646] dark:text-[#E39A78]`，仅"实时刚完成"时 `chat-motion-pop` 弹入（`StatusIcon` `1300-1332`，`justCompleted` 门控避免历史批量弹动）。
  - error/skipped/cancelled: 中性灰 lucide 图标（`AlertCircle`/`CircleSlash`/`XCircle`），**无强调色、无动效**。
  - 展开/折叠: `chat-motion-reveal`（`1441`）+ chevron `rotate-180`（`1435`）。
- **对照 LiveAgent**: 工具卡是**磨砂卡片**（`--tool-card-bg` 带 alpha + blur），**每类工具左边一条 accent 色条/图标着色**，映射在 `assistantBubbleUtils.ts:42-79`（`{Icon, accent, category}`：bash/server→绿、file/read/write/edit/delete→蓝、search/grep→橙、list→紫）。
- **差异**: Kivio 工具行完全中性灰阶（这是 STYLE.md 的刻意纯灰阶取向，见 `ToolCallBlock.tsx:381-383` 注释）；LiveAgent 用领域色区分工具类别。

### 1.3 推理块 ReasoningBlock（thinking）
- **组件**: `src/chat/ReasoningBlock.tsx`（全 185 行）
- **现状（已相当完整）**:
  - 标题 streaming 时 `reasoning-shimmer-text` 灰色流光（`ReasoningBlock.tsx:126`），非流式静态。
  - 正文 `chat-motion-reasoning-body` max-height 过渡（`89`）+ 折叠态 `is-collapsed`/`is-open`。
  - 流式尾部 `reasoning-stream-tail`（`reasoning-tail-pulse` 0.22s，`96` + css `195-206`）每次新内容脉冲。
  - 滚动框上下渐隐遮罩 `reasoning-scroll-frame`（css `242-298`）。
  - 左边框 streaming/静态两档色（`110-114`）；chevron rotate。
  - 实时计时 `liveDurationMs`（`54-62`）。
- **对照 LiveAgent**: 思考区用 `--chat-thinking-bg/border` 令牌上底色。
- **差异**: 观感已到位；Kivio 无 thinking 专属底色令牌（当前用 border-left + 灰字，视觉更轻）。**shimmer 现状已达标，无缺口。**

### 1.4 子代理卡 sub-agent card
- **组件**: `src/chat/ToolCallBlock.tsx` → `SubAgentCard`（`487-567`），共享外壳 `ConsultCard`（`422-477`）。
- **现状（已相当完整）**:
  - 卡片有边框 + 底色 + hover 过渡：`rounded-md border border-neutral-200 bg-neutral-50/70 … hover:bg-neutral-100/70 transition-colors duration-200`（`ConsultCard` `441`）。
  - eyebrow 7×7 方块，running 时 `kv-card-eyebrow--running` 光标闪烁（`CardEyebrow` `371-379`）。
  - statusLine running 时 `reasoning-shimmer-text` 流光（`ConsultCard:463`）。
  - 另有 `subagent-sparkle`（字符摩点星芒，css `1588-1636`）与 `chat-motion-subagent-shimmer`（紫流光，css `1380`）作为备用原语 —— 注意：`SubAgentCard` 当前走 `ConsultCard` 用的是 `kv-card-eyebrow` + `reasoning-shimmer-text`，**`subagent-sparkle` / 紫色 `chat-motion-subagent-shimmer` 在 `src/chat/` 内无 TSX 引用**（grep 仅命中 css 定义），疑为历史遗留或他处（如 MessageGroup 分组头）使用，需实施时确认是否死代码。
- **对照 LiveAgent**: 用 `--chat-running`（紫 `252 56% 57%`）表达运行态。
- **差异**: Kivio 子代理卡已有磨砂感雏形（半透明底 + hover），运行态表达充分。缺口在于 running 强调色是灰阶 shimmer，非专属"running 紫"。

### 1.5 上下文压缩 compaction UI
- **组件**: `src/chat/CompactionDivider.tsx`(82) / `CompactionInProgress.tsx`(18) / `CompactionSummaryPanel.tsx`(40)
- **现状（已完整）**:
  - divider: `chat-compaction-divider--animate`（680ms，线条 scaleX 展开 + 内容上移，css `1441-1471`），`--pre-enter` 预入场态。
  - 进行中: `CompactionInProgress` = `chat-compaction-progress`（320ms 淡入）+ `chat-motion-soft-pulse` + label `chat-motion-tool-shimmer`（`CompactionInProgress.tsx:10-12`）。
  - 摘要面板: `CompactionSummaryPanel` 用 `chat-motion-reveal`（`CompactionSummaryPanel.tsx:33`）+ chevron 旋转。
- **差异**: **无缺口**，动效编排已细致。

### 1.6 jump-to-bottom 按钮
- **组件**: `src/chat/MessageList.tsx:651-661`
- **现状**: `!atBottom` 时渲染圆形按钮，入场用 `chat-motion-pop`（spring 弹入），hover/active `active:scale-90`，`bg-white/95 … backdrop-blur`（已有轻磨砂）+ `shadow-md`。
- **对照 LiveAgent**: `.chat-jump-to-bottom` 磨砂更重（`blur(18px) saturate(180%)` + inset 高光 + 主题 tint 半透明底），入场 `chatJumpToBottomIn`（translateY(6px) fade，非 pop）。
- **差异**: Kivio 已磨砂 + 弹入，接近达标；LiveAgent 磨砂质感更强（inset 高光 + saturate + 更大 blur）。

---

## 2. 差距清单

| # | 差距 | 涉及文件 (file:line) | 可复用 Kivio 原语 | 预估改动量 |
|---|---|---|---|---|
| B1 | 气泡入场缺 `scale(0.98)` 微缩（当前仅 translateY） | `src/index.css:1301-1310` `@keyframes chat-motion-fade-up` | 直接改 keyframe，或新增变体 | **XS**（1 处 keyframe 加 transform；影响所有 `chat-motion-fade-up` 使用点，需评估附件 chip 等是否合适） |
| B2 | 工具行图标/状态无按类型强调色（全中性灰） | `src/chat/ToolCallBlock.tsx:107-166`(`toolGlyph`)、`1334-1378`(`ToolTypeIcon`)、`1300-1332`(`StatusIcon`) | 需新增领域色令牌（见 §3） | **M**（令牌 + `toolGlyph` 返回值扩展 accent + 图标着色；见 §3 高价值评估） |
| B3 | error/cancelled 状态无强调色、无动效（仅静态灰图标） | `src/chat/ToolCallBlock.tsx:1322-1331`、`1343-1351` | `--chat-error` 令牌 + `chat-motion-pop`（error 出现时弹入） | **S**（加 error 令牌 + 复用现有 pop） |
| B4 | 子代理/工具 running 态无专属"running 色"（灰阶 shimmer） | `src/chat/ToolCallBlock.tsx:463`、css `chat-motion-tool-shimmer`(`1406`)、`chat-motion-subagent-shimmer`(`1380` 疑未接入) | `chat-motion-subagent-shimmer` 已存在（紫），可直接接回子代理卡 | **S**（把已有紫 shimmer 接到 `ConsultCard` running statusLine，替换灰 shimmer） |
| B5 | jump-to-bottom 磨砂质感弱于参考（无 inset 高光/saturate） | `src/chat/MessageList.tsx:657` | 参照 `.window-frosted`(`index.css:160-179`) 的 inset 高光配方 | **XS**（改一处 className / 新增 `.chat-jump-to-bottom` class） |
| B6 | `subagent-sparkle` / 紫 `chat-motion-subagent-shimmer` 疑为死代码 | css `1380-1402`、`1588-1636`；`src/chat/*` 无 TSX 引用 | — | **XS 调查**（确认后决定接回 B4 或删除；勿盲目新增重复星芒） |

---

## 3. 重点评估：是否值得引入 LiveAgent 式「领域色令牌 + 每类工具强调色」

### 证据：Kivio 当前颜色怎么来的 —— **散落硬编码，无令牌层**
- `grep --chat-* / --tool-*-accent` 在 `src/index.css` **零命中** —— Kivio **没有任何领域色/工具色令牌**。
- 颜色全是**内联字面量**，散落在组件与 css：
  - 完成 ✓ terracotta：`text-[#C56646] dark:text-[#E39A78]`（`ToolCallBlock.tsx:1316`）+ css 注释同源硬编码（`index.css:1405`、shimmer `rgba(197,102,70,1)` `1411`）。
  - 子代理紫：`rgba(139,92,246,…)`（`index.css:1383-1385`）。
  - 警告 terracotta 变体：`#e8a090`/`#a35f51`/`#f1b4a7`（`InputBar.tsx:1580` 等）、`chat-motion-soft-pulse` 里 `rgba(232,160,144,…)`（`index.css:1349-1353`）。
  - 工具图标/名/状态：一律 `text-neutral-400/500/700`（`ToolCallBlock.tsx` 通篇）。
- 即"**同一 terracotta 品牌色在至少 4 处各写各的**"，改品牌色需多点同步。

### 对照：LiveAgent 集中化
- 全部走 HSL 令牌（`--chat-user/assistant/tool/thinking/success/error/running` + `--tool-{bash,file,search,list}-accent` + `--tool-card-bg`，`LiveAgent index.css:187-206` 亮 / `261-281` 暗）。
- 工具→accent 映射集中在 `assistantBubbleUtils.ts:42-79`（`{Icon, accent, category}`）。组件从不写死颜色，只引 `var(--…)`。

### 结论（高价值，但需分层拿捏）
1. **令牌化 Kivio 现有品牌色（terracotta ✓ / 子代理紫 / running / error / warning）= 高价值、低风险**：把散落的 `#C56646`/`rgba(139,92,246)` 等收敛成 `--kv-chat-*` 令牌，**不改任何视觉**，只改组织方式。这是本组最稳的收益，直接消除多点同步隐患。**建议做。**
2. **每类工具一个强调色（bash 绿 / file 蓝 / search 橙 / list 紫）= 中价值、需谨慎**：
   - 基础设施已就位 —— `toolGlyph()`（`ToolCallBlock.tsx:107`）已是集中的"工具→图标"映射点，扩成"工具→{图标, accent}"是自然低成本改动（对齐 LiveAgent 的 `assistantBubbleUtils.ts`）。
   - **但**与 Kivio 现有刻意的"纯灰阶工具行"取向冲突（`ToolCallBlock.tsx:381-383` 明确注释"保持 STYLE.md 纯灰阶调色板"）。引入四色会改变视觉身份基调 —— **属于视觉决策，超出"只补交互质感"的约束**，应作为可选项交主代理/设计决策，不建议默认实施。折中：只给**图标**极轻着色（低饱和），工具名/背景仍灰。
3. **不要引入 `--chat-user-bg`/`--chat-assistant-bg` 式的气泡底色令牌**：Kivio 气泡走 `--theme-surface*` 体系（warm/cool 主题，`index.css:116-136`），已有自己的表面令牌层，引入 LiveAgent 那套会与主题系统打架 —— **已有等价物，勿重复造**。

---

## 4. 已有等价物 / 勿重复造

| LiveAgent 概念 | Kivio 已有等价物 |
|---|---|
| `chatBubbleIn` | `chat-motion-fade-up`（`index.css:1052`，仅缺 scale） |
| `chatJumpToBottomIn` + 磨砂 | `chat-motion-pop` + `backdrop-blur`（`MessageList.tsx:657`，磨砂较轻） |
| `--chat-running` 紫运行态 | `chat-motion-subagent-shimmer` 紫（`index.css:1380`，疑未接入）+ `kv-card-eyebrow--running` |
| 工具卡磨砂 `--tool-card-bg` | `ConsultCard` 半透明底 + hover（`ToolCallBlock.tsx:441`）+ 全局 `.window-frosted` 磨砂配方（`index.css:160`） |
| thinking shimmer | `reasoning-shimmer-text`（`index.css:218`）+ `reasoning-stream-tail` |
| 表面色令牌 | `--theme-surface*`（warm/cool 主题，`index.css:116-136`） |
| reduced-motion 兜底 | 已完整（`index.css:1648-1705`） |

---

## Caveats / Not Found
- **B6 待确认**：`subagent-sparkle`（css `1588-1636`）与紫 `chat-motion-subagent-shimmer`（css `1380`）在 `src/chat/**/*.tsx` 内无引用（grep 仅命中 css）。可能被 `MessageGroup.tsx` / 分组头或 Kivio Code TUI 侧使用，也可能是历史死代码 —— 实施前需再 grep 全仓（含 `src/` 之外）确认，避免误删或重复新增。
- 未审：`MessageGroup.tsx`(365) 的分组头动效（`kv-tick-spinner` 等生成中指示器在此，`index.css:1188-1199`）—— 本组聚焦气泡/工具卡/推理块，分组头 spinner 已有实现，未逐行展开。
- LiveAgent 的 `--chat-*` 是 HSL 三元组（配合 `hsl(var(--x))`）；Kivio 现有令牌是完整颜色值/cubic-bezier。若令牌化，需选定与 Kivio 现有风格一致的写法（完整值），勿照搬 HSL 三元组约定。
- 视觉决策（是否引入四色工具强调）不属研究结论，已在 §3 标为"交主代理/设计决策"。
