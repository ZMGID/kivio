# Research: 动效审计 组 A — 入口 / 空态 / 输入区

- **Query**: 盘点 Kivio Chat 入口/空态/输入区 surface 的入场与交互动效现状，对照 LiveAgent 找缺口（只读只记录）
- **Scope**: internal（Kivio）+ 参考（LiveAgent index.css）
- **Date**: 2026-07-18

## 概览：Kivio 已有的动效原语（可复用，勿重复造）

全部定义在 `src/index.css`。

**缓动令牌**（`src/index.css:81-90`）
- `--kv-ease-standard` = `cubic-bezier(0.22, 1, 0.36, 1)` — 主入场/展开/弹出曲线（干脆 + 轻微回弹尾）
- `--kv-ease-firm` = `cubic-bezier(0.2, 0, 0, 1)` — 拖拽/重排
- `--kv-ease-spring` = `cubic-bezier(0.34, 1.56, 0.64, 1)` — 轻弹/按压回弹（**与 LiveAgent logo pop 的曲线完全相同**）
- `--kv-ease-out` = `cubic-bezier(0.33, 1, 0.68, 1)` — 颜色/透明度收尾
- 时长：`--kv-dur-instant` 90ms / `--kv-dur-fast` 150ms / `--kv-dur-normal` 220ms / `--kv-dur-slow` 320ms

**入场/交互 class**
| Class | 效果 | 定义 |
|---|---|---|
| `chat-motion-fade-up` | opacity 0→1 + translateY(8px→0)，**读 `--chat-motion-delay` + `--chat-motion-duration`** | `src/index.css:1052`, keyframe `:1301` |
| `chat-motion-popover` | opacity 0→1 + translateY(`--chat-popover-start-y`,-3px) + scale(0.985→1)，**读 `--chat-popover-origin`** | `src/index.css:1057`, keyframe `:1325` |
| `chat-motion-row` | opacity 0→1 + translateX(-4px→0)，**读 `--chat-motion-delay`** | `src/index.css:1062`, keyframe `:1336` |
| `chat-motion-pop` | 弹性 pop（`--kv-ease-spring`） | `src/index.css:1264`, keyframe `:1250` |
| `chat-motion-fade` | 纯淡入（`--kv-dur-fast`） | `src/index.css:1229` |
| `chat-motion-soft-pulse` | **常驻 idle 脉冲**（box-shadow + scale 1↔1.015，1.8s 循环） | `src/index.css:1067`, keyframe `:1347` |
| `chat-model-stack-item` / `@chat-model-stack-in` | scale(0.4→1) 弹入（`--kv-ease-spring`） | `src/index.css:1072-1085` |
| `chat-motion-modal-in` | 模态入场 | `src/index.css:1245` |
| `chat-motion-reveal` | grid-rows 0fr→1fr 折叠展开 | `src/index.css:1091` |
| `chat-motion-exit` | 退场（用于附件移除） | `src/index.css:1214` |

**逐项 stagger 机制已存在**：`chat-motion-fade-up` 与 `chat-motion-row` 都读 `--chat-motion-delay`。已在 `Sidebar.tsx:1060` / `Sidebar.tsx:1203` 用 `style={{ '--chat-motion-delay': \`${Math.min(index,12)*18}ms\` }}`（18ms/项，封顶 12 项）落地会话列表级联。这就是 LiveAgent `mentionItemIn` 18ms 级联的现成等价物。

**reduced-motion 兜底**：`src/index.css:1648-1720` 已覆盖全部上述 class（含 `chat-motion-fade-up`/`popover`/`row`/`soft-pulse`），全局 `animation-duration:0.01ms` 兜底 + 具名 `animation:none`。**新增动效若复用现有 class 自动被兜底；若加新 keyframe 需手动补进该块。**

---

## 逐 surface 现状

### Surface 1 — 空态 / 欢迎页 / hero
**组件文件**：`src/chat/Chat.tsx:3621-3703`（`showEmptyHero` 分支）；`src/chat/ChatDotGridBackground.tsx`（背景 canvas）；`src/chat/TypewriterText.tsx`（标题打字机）；`src/chat/InputBar.tsx`（内联 composer）
**greeting 文案**：`Chat.tsx:3350-3356`（`pickRandomChatEmptyGreeting()`，随会话 id 变化）

现状：
- 容器 `.chat-empty-hero`（`src/index.css:1009`）内含 `<ChatDotGridBackground/>`（`Chat.tsx:3623`）——**点阵 canvas，每点明暗呼吸的常驻 idle 动效**（rAF loop `ChatDotGridBackground.tsx:334`；CSS 注释 `src/index.css:1008`）。
- 内容栈 `.chat-empty-hero-stack .chat-motion-fade-up`（`Chat.tsx:3624`）——**整个栈作为单一单元一次性 fade-up，无编排序列、无 stagger**。
- 标题 `<h2 .chat-empty-hero-title>`（`Chat.tsx:3625`）内嵌 `<TypewriterText>`（`Chat.tsx:3640`）——逐字打字机 + `chat-typewriter-cursor` 闪烁（`src/index.css:1035`）。
- 栈内只有：**标题 h2 + InputBar（`Chat.tsx:3647`）**。**无 logo、无副行/CTA、无建议卡/快捷入口**。

结论：Kivio hero 是「打字机标题 + 输入框」的极简形态，**没有 LiveAgent 那种 logo→标题→副行→CTA→建议卡的分层编排**；idle 氛围由背景点阵呼吸承担（而非 logo float/halo breathe）。

### Surface 2 — 输入区 composer（InputBar）
**组件文件**：`src/chat/InputBar.tsx`；附件 `src/chat/ChatAttachments.tsx`
现状（交互质感已较完整）：
- **发送/停止按钮**（`InputBar.tsx:1780-1818`）：同槽位 send↔stop 双按钮，opacity + `scale-90` crossfade，用 `--kv-dur-fast` + `--kv-ease-spring`，`active:scale-90` 按压反馈；`canSend` 且非取消态时挂 **`chat-motion-soft-pulse` 常驻脉冲**（`InputBar.tsx:1796`）。
- **附件 chip**：入场 `chat-motion-fade-up`、移除 `chat-motion-exit`（`ChatAttachments.tsx:115`、`:189`）；图片预览 `chat-motion-fade`（`ChatAttachments.tsx:63-64`）。
- **提示行**（拖拽/错误/警告）：`chat-motion-fade-up`（`InputBar.tsx:1580, 1585, 1594, 1599`）。
- 模式菜单 chevron 旋转 `rotate-180`（`InputBar.tsx:1726`）。
- **无** LiveAgent 的 `composerTypeCharIn`（输入字符 blur-in）——但 Kivio 已在 hero 标题用打字机，composer 本身无此需求。

### Surface 3 — mention 弹窗 / @ 补全列表
**现状：Kivio 没有 @ mention / @ 补全功能**（`grep` 确认 `InputBar.tsx` 无 mention/@ 触发逻辑）。功能性对应缺失。
可类比的「弹层 + 列表」surface（LiveAgent mention 模式的等价落点）：
- **Slash 命令面板** `InputBar.tsx:1377-1438`：面板 `chat-motion-popover`；列表项（`filteredSlashCommands.map`，`:1391`）**无逐项 stagger**。
- **项目选择菜单** `InputBar.tsx:1439-1499`：面板 `chat-motion-popover`；项 `visibleProjectOptions.map`（`:1472`）**无 stagger**。
- **模式菜单** `InputBar.tsx:1731-1775`：`chat-motion-popover`；项 **无 stagger**。
- **Skill/工具面板** `InputBar.tsx:1324-1376`：`chat-motion-popover`。
- **专家（Assistant）选择器** `AssistantPicker.tsx:82-146`：`createPortal` 弹层 `chat-motion-popover`（`:87`）；项 `assistants.map`（`:107`）**无 stagger**。

所有弹层都已用 `chat-motion-popover`（= LiveAgent `mentionPopupIn` 的等价物：opacity + translateY + scale + transform-origin）。**唯一缺口是列表项没有逐项进入的级联。**

### Surface 4 — 顶部 model 选择器 / skill 选择器（下拉入场）
**组件文件**：`src/chat/ModelSelector.tsx`、`src/chat/MultiModelSelector.tsx`、`src/chat/ThinkingLevelSelector.tsx`、`src/chat/RuntimePicker.tsx`、`src/chat/PermissionPicker.tsx`；Skill 相关 `src/chat/SkillCenter.tsx`
现状：
- `ModelSelector.tsx:167` 菜单 `chat-model-selector-menu chat-motion-popover`；模型行（`favoriteEntries.map`/`models.map` `:174-184`，`renderModelRow`）**无逐项 stagger**。
- `MultiModelSelector.tsx:112`、`ThinkingLevelSelector.tsx:101`、`RuntimePicker.tsx:269`、`PermissionPicker.tsx:122`：均 `chat-model-selector-menu chat-motion-popover`，**项无 stagger**。
- 多模型头像堆叠已有 `chat-model-stack-item`（scale 弹入，`src/index.css:1072`）。
- Skill「选择器」的内联入口即 InputBar 工具面板（Surface 3 的 `:1324`）；`SkillCenter` 本身是整页/模态（`SkillCenter.tsx:679` `chat-motion-modal-in`，高级区 `:535` `chat-motion-reveal`）——模态入场已有动效。

---

## 差距清单（LiveAgent 有 / Kivio 缺）

> 参考实现：`/Users/zmair/ZM database/cankao/LiveAgent/crates/agent-gui/src/index.css`

### G1 — 弹层列表项逐项级联（mentionItemIn / composerDropdownItemIn）
- **LiveAgent**：`mentionItemIn`（`index.css:1417`）按 nth-child ~18ms/项（`:1438-1464`）；`composerDropdownItemIn`（`:1489`）MCP/Skills 项 ~20ms/项（`:1518-1546`）。
- **Kivio 现状**：所有弹层项无 stagger（见 Surface 3/4）。
- **涉及文件**：`InputBar.tsx`（slash `:1391`、项目 `:1472`、模式 `:1740`）、`AssistantPicker.tsx:107`、`ModelSelector.tsx:174/184`、`MultiModelSelector.tsx`、`ThinkingLevelSelector.tsx`、`RuntimePicker.tsx`、`PermissionPicker.tsx`。
- **可复用原语**：`chat-motion-fade-up` 或 `chat-motion-row`（都读 `--chat-motion-delay`）+ `Sidebar.tsx:1060` 已验证的 `index*18ms` 内联写法。
- **预估改动量**：**CSS + 少量 tsx**（每处 map 里给项加 class + `style={{'--chat-motion-delay': ...}}`）。若想纯 CSS，可仿 LiveAgent 加 `:nth-child` 规则，但 Kivio 现行是 JSX 传 delay 的模式，跟随更一致。

### G2 — 空态 hero 分层编排（chatHeroRiseIn 序列）
- **LiveAgent**：logo pop（`chatHeroLogoIn` 0.55s spring，`index.css:1019`）→ 标题 rise（delay 0.12s，`:1034`）→ 副行（0.2s，`:1039`）→ CTA（0.28s，`:1043`）→ 建议卡级联（`--chat-hero-delay` 从 JSX，`:1049`）；用 `backwards` 让入场后 hover/idle 接管。
- **Kivio 现状**：整栈单一 `chat-motion-fade-up`（`Chat.tsx:3624`），标题与 InputBar 同时入场，无先后。
- **涉及文件**：`Chat.tsx:3624-3701`（拆分 stack 的单一 class 为标题、composer 两级 delay）。
- **可复用原语**：`chat-motion-fade-up`（已支持 `--chat-motion-delay` / `--chat-motion-duration`），无需新 keyframe。
- **预估改动量**：**CSS-only 到 CSS + 极少 tsx**（把 stack 的 `chat-motion-fade-up` 下放到标题/composer 各自 + 传两个 delay）。注意 Kivio 无 logo/副行/CTA，只能编排「标题→输入框」两级。

### G3 — logo 弹性 pop + idle float + halo breathe
- **LiveAgent**：`chatHeroLogoIn`（spring）入场 →`chatHeroFloat`（6s ±5px，`:1065`）+ `chatHeroHaloBreathe`（opacity 1↔0.6，`:1075`），delay 1.2s 让 pop 先落定。
- **Kivio 现状**：**hero 无 logo**，故此项无直接落点。idle 氛围由 `ChatDotGridBackground` 点阵呼吸承担（等价的「常驻低频呼吸」已存在）。
- **结论**：**多数 N/A**。若未来 hero 引入 logo，spring pop 可直接用 `--kv-ease-spring` + `chat-motion-pop`/`chat-model-stack-in`；float/breathe 需新增 2 个 idle keyframe（并补 reduced-motion 兜底）。当前不建议为了对齐而硬加 logo（会改视觉身份）。

### G4 —（可选）建议卡 / 快捷入口级联
- **LiveAgent**：`chatHeroCardIn`（`index.css:1096`）建议卡逐卡 `--chat-hero-delay`。
- **Kivio 现状**：**空态无建议卡这一内容**。这是**功能/内容缺口**，非纯动效。
- **预估改动量**：**需新增功能（内容 + 组件），超出「只补交互质感」范围** —— 标记为需产品决策，不属于本次动效打磨可 CSS 化的项。

### G5 —（可选）composer 字符 blur-in（composerTypeCharIn）
- **LiveAgent**：`composerTypeCharIn`（`index.css:1394`）输入字符 blur→clear。
- **Kivio 现状**：无；但 hero 标题已有 `TypewriterText`。
- **预估改动量**：非必要（Kivio 已有打字机语汇），建议跳过。

---

## Kivio 其实已有等价物（避免重复造）

| LiveAgent 模式 | Kivio 等价物 | 位置 |
|---|---|---|
| `mentionPopupIn`（opacity+translateY(6)+scale(0.98)+origin） | `chat-motion-popover`（opacity+translateY+scale(0.985)+`--chat-popover-origin`） | `src/index.css:1057/1325`（已用于全部弹层） |
| `chatHeroRiseIn` / `modelSelectorItemIn`（fade + rise） | `chat-motion-fade-up` | `src/index.css:1052/1301` |
| `chatHeroLogoIn`（spring pop 曲线 0.34,1.56,0.64,1） | `--kv-ease-spring` + `chat-motion-pop` / `chat-model-stack-in` | `src/index.css:83, 1264, 1076` |
| 逐项 stagger delay 机制（nth-child / `--chat-hero-delay`） | `--chat-motion-delay`（`chat-motion-fade-up`/`row` 已读） + `Sidebar.tsx:1060` 的 `index*18ms` 写法 | `src/index.css:1054/1064` |
| idle 常驻呼吸（halo breathe / float） | 发送键 `chat-motion-soft-pulse` + hero 背景 `ChatDotGridBackground` 点阵呼吸 | `src/index.css:1067`；`ChatDotGridBackground.tsx` |
| `composerDropdownIn`（下拉入场） | `chat-motion-popover` | 全部下拉已用 |
| reduced-motion 分支 | 已有完整兜底块 | `src/index.css:1648-1720` |

**净结论**：LiveAgent 该组的「弹层/入场形态」Kivio 基本都已有对应原语并已铺开；真正可低成本补齐的交互质感只有 **G1（弹层列表项级联，CSS+少量 tsx）** 和 **G2（空态标题→输入框两级编排，CSS-only/极少 tsx）**。G3/G4 因 Kivio hero 无 logo/建议卡，属 N/A 或需产品决策，不应为对齐参考而改动视觉身份。

## Caveats / 未覆盖
- 未逐一核对每个下拉的 `renderModelRow` 内部结构是否便于挂 stagger（`ModelSelector.tsx` 有分组 favorite/grouped 两段，级联序需按渲染顺序统一计 index，可能要跨段累加）。
- `TypewriterText.tsx`、`ChatDotGridBackground.tsx` 仅确认了存在与用途（打字机 / rAF 呼吸），未逐行核对其 reduced-motion 处理（`chat-typewriter-cursor` 在 `src/index.css:1716` 已 `display:none` 兜底）。
- LiveAgent 的 model/mention 弹层由 Base UI / Radix 的 `data-state`/`data-starting-style` 驱动；Kivio 是手写条件渲染 + `chat-motion-popover` 入场（无对称的退场动画）——若要对齐退场动效属额外工作，本次未纳入差距清单。
