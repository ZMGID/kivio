---
name: Kivio Desktop
description: "安静的工作台：克制、清晰、紧凑的桌面 AI 客户端"
colors:
  surface: "#fdfcfa"
  surface-soft: "#f9f8f6"
  surface-muted: "#f4f3f1"
  surface-hover: "#eeedeb"
  surface-active: "#e7e6e4"
  surface-titlebar: "#eae9e7"
  border: "#e2e1df"
  border-strong: "#d2d1cf"
  text: "#1d1d1f"
  text-muted: "#737373"
  text-faint: "#9c9ca3"
  accent: "#2f6ff0"
  accent-hover: "#2960d8"
  accent-soft: "#e6efff"
  on-accent: "#ffffff"
  danger: "#c4341c"
  danger-soft: "#fdecea"
  dark-surface: "#212121"
  dark-surface-soft: "#262629"
  dark-surface-muted: "#2a2a2d"
  dark-surface-hover: "#2c2c30"
  dark-surface-active: "#36363c"
  dark-surface-titlebar: "#313136"
  dark-border: "#3a3a40"
  dark-border-strong: "#4e4e56"
  dark-text: "#f0f0f3"
  dark-text-muted: "#a1a1aa"
  dark-text-faint: "#6e6e76"
  dark-accent: "#5c8df7"
  dark-accent-hover: "#6f9bf8"
  dark-accent-soft: "#2a3243"
  dark-on-accent: "#ffffff"
  dark-danger: "#ef6a5a"
  dark-danger-soft: "#3e2b29"
typography:
  headline:
    fontFamily: "system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", \"PingFang SC\", \"Microsoft YaHei\", \"Hiragino Sans GB\", sans-serif"
    fontSize: "1.375rem"
    fontWeight: 500
    lineHeight: 1.2
    letterSpacing: "-0.02em"
  title:
    fontFamily: "system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", \"PingFang SC\", \"Microsoft YaHei\", \"Hiragino Sans GB\", sans-serif"
    fontSize: "20px"
    fontWeight: 600
    letterSpacing: "-0.02em"
  body:
    fontFamily: "system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", \"PingFang SC\", \"Microsoft YaHei\", \"Hiragino Sans GB\", sans-serif"
    fontSize: "13px"
    lineHeight: 1.4
  label:
    fontFamily: "system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", \"PingFang SC\", \"Microsoft YaHei\", \"Hiragino Sans GB\", sans-serif"
    fontSize: "12px"
    fontWeight: 500
    lineHeight: 1.4
  mono:
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, \"Cascadia Mono\", \"PingFang SC\", \"Microsoft YaHei\", \"Hiragino Sans GB\", monospace"
rounded:
  nav-standalone: "5px"
  control: "6px"
  icon-md: "7px"
  nav: "8px"
  menu: "10px"
  card: "12px"
  shell: "14px"
  frosted: "18px"
  pill: "999px"
spacing:
  gap-tight: "4px"
  gap-control: "5px"
  gap-small: "8px"
  inset-small: "12px"
  inset-card: "16px"
  gutter-standalone: "20px"
  gutter-embedded: "28px"
components:
  button-default:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "4px 11px"
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.on-accent}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "4px 11px"
  button-ghost:
    backgroundColor: "transparent"
    textColor: "{colors.text}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "4px 11px"
  button-danger:
    backgroundColor: "transparent"
    textColor: "{colors.danger}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "4px 11px"
  button-primary-hover:
    backgroundColor: "{colors.accent-hover}"
    textColor: "{colors.on-accent}"
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.control}"
    padding: "5px 9px"
  settings-card:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.card}"
    padding: "4px 16px 8px"
---

# Design System: Kivio Desktop

## Overview

**Creative North Star: "安静的工作台"**

Kivio 桌面客户端以克制、清晰、紧凑为共同气质。系统字体、低饱和表面、细边框与局部交互强调让界面退后，让对话、工具和当前任务成为主角。

控件紧凑、圆润、低干扰；平面为主，浮层抬升。文档描述现有系统，不把某个页面的构图推广为全局规则，也不将默认蓝色固定为所有主题的品牌色。

范围：React / Tauri 桌面 WebView 客户端及其共享控件，不覆盖官网。官网保留独立的 [website/STYLE.md](website/STYLE.md)。产品事实见 [PRODUCT.md](PRODUCT.md)，工程复用要求以 [统一工程规范](docs/engineering-standards.md) 为准。

**Key Characteristics:**
- 系统字体与 CJK 回退；代码使用等宽字体。
- 语义主题色支持明暗与自定义色板。
- 紧凑控件与桌面容器响应式布局。
- 色阶与细边框分区，菜单和弹窗用阴影表达浮层。

## Colors

默认 Neutral 主题用纸白、墨灰、清晰蓝描述；frontmatter 是该主题完整的明暗基准，带 dark- 前缀的条目是深色值。它不替代运行时主题选择。

### Primary

- **清晰蓝（accent / accent-hover / accent-soft）**：主动作、选中标记与聚焦反馈；文字使用 on-accent。其他主题可以改变该角色的色相。
- **高对比主色（text / surface）**：Streamdown 的 --color-primary 映射主要文字，不是交互强调色。

### Neutral

- **纸白与分层表面**：surface 为主内容，surface-soft / surface-muted 为次级区域，surface-hover / surface-active 为交互状态，surface-titlebar 为标题栏与部分分隔角色。
- **墨灰文字**：text / text-muted / text-faint 区分主要、次要与更弱的信息；border / border-strong 区分表面边界与控件边界。
- **危险状态**：danger / danger-soft 用于危险操作与反馈，不作为第二品牌强调色。设置内成功和警告有独立的局部语义变量，未纳入 17 字段主题契约。

**The Theme Roles Rule.** 默认色板是基准样本；界面实现必须引用现有语义变量，随用户主题切换。

来源：[theme.ts](src/theme/theme.ts)、[types.ts](src/theme/types.ts)、[index.css](src/index.css)。运行时 Neutral 的 text-muted 与启动兜底不同；本档采用运行时定义。共有十二套内置主题：neutral、warm、cool、graphite、blossom、grove、ocean、ember、iris、white、nord、solarized；其他色板继续由 theme.ts 维护，不在此复制。

配套 [.impeccable/design.json](.impeccable/design.json) 保存阴影、动效、断点、组件预览与叙述扩展。其八阶 OKLCH 色带是为面板合成的展示辅助，不是从代码提取的主题色阶，不得用作新增界面的规范色板。预览是无框架的外观片段，不实现设置写入或业务行为；按钮聚焦采用聊天宿主示例，不代表独立设置宿主。

## Typography

系统无衬线是界面字体；--font-serif 也映射同一系统栈，不代表另有展示衬线。全局等宽栈保留 CJK 无衬线回退。不另设品牌展示字体，也不存在单一数学比例的字号系统。

- **Headline**：聊天空态标题，使用 frontmatter 的 headline；低高度容器会缩小。
- **Title**：嵌入式设置页标题使用 title；独立设置标题保留较小规格（16px / 600），不是另一套品牌字体。
- **Body / Label**：设置正文与控件标签使用对应角色；聊天导航常用正文级字号，编辑输入另用（15px / 1.75）。辅助说明存在（11.5px）与元信息（11px）等局部规格，不强行统一。
- **Mono**：代码及技术内容使用等宽字体；部分已有输入框与芯片有更短的局部等宽栈，应按现有组件使用。

macOS 使用灰度抗锯齿；Windows 保持默认渲染，避免关闭 ClearType。来源：index.css、settings-01-embedded.css、settings-02-pages.css、chat-01-main.css。

## Layout

桌面外壳为侧栏、主区与可选右侧 Dock；主阅读列上限（53rem）。侧栏默认（240px），可调整（200–400px）；Dock 默认（360px），可调整（320–560px）。本档 spacing 是重复出现的提取值，不意味着代码已有全局间距 token。

设置的嵌入式导航（200px）与独立导航（172px）是现有宿主差异；引导侧栏另为（208px）。普通设置内容上限（820px），会话设置允许（1200px）；嵌入式内容边距使用 gutter-embedded，独立宿主使用 gutter-standalone。

响应主要取决于容器，而非移动端产品假设：settings-shell 在（820px）以内将嵌入导航改为（52px）图标栏，在（720px）以内堆叠供应商区域，在（560px）以内收窄边距；独立导航在（480px）以内缩为图标栏。chat-main 在（560px）以内收窄输入和消息边距，在（520px / 480px）以内逐级隐藏非关键标签；消息与标题导航各有（719px / 959px）阈值。详细断点见 sidecar；已有部分设置页使用视口媒体查询，不将其描述为全仓统一规则。

滚动发生于内容容器；custom-scrollbar 使用细胶囊式滚动条（8px 槽，3px 透明边），不把滚动转移到整个文档。

## Elevation & Depth

平面为主，浮层抬升。区域主要通过表面色阶和细边框区分；菜单、选择列表、弹窗与输入区域使用轻阴影表达可操作性或覆盖关系。并非所有表面都使用毛玻璃。

阴影保留现有作用域差异：root 的 shadow-sm 是主题文字混色，设置 .kv 的 shadow-sm / shadow-md 使用自身明暗值；深色嵌入设置卡片取消阴影。菜单采用多层柔和投影，window-frosted 使用饱和度与模糊（160% / 28px）。准确值在 sidecar 的 extensions.shadows 中，不复制为新的 CSS 来源。

动效引用全局 --kv-ease-* 与 --kv-dur-*；目前仍有部分控件使用局部时长，不声称已全部统一。减少动态效果规则位于 chat-01-main.css，将时长压至（0.01ms），保留结束事件，取消按钮按压缩放。侧栏布局瞬时切换，仅透明度淡出，不动画 margin-left。

html、body 和 root 保持透明，让原生材质与截图浮窗继续工作。浏览器预览不能证明原生材质效果。

## Shapes

控件与区域使用分层圆角，不是一刀切：control 用于按钮、输入与选择器；nav 用于嵌入导航；menu 用于浮层；card 用于嵌入设置分组；shell 用于聊天外壳；frosted 用于独立轻量窗口；pill 用于芯片与状态标签。最大化及平台标题栏模式可将外壳圆角归零，保留已有平台分支。

## Components

**The Shared Owner Rule.** 共享控件的视觉规则由现有组件与全局样式维护，页面只负责内容与布局。

### Buttons

紧凑、圆润、低干扰。[Button / IconButton](src/components/Button.tsx) 是动作按钮入口。Button 提供 default、primary、ghost、danger，以及 md / sm；小号规格（2px 8px，11.5px）。普通按压缩放（0.97），禁用透明度（0.5）。

IconButton 默认 sm（28px），另有 xs / md / lg（22px / 32px / 36px）与 circle；图标带 aria-label 和 title。ghost 复用透明基底，静止图标使用 text-muted，悬停沿用既有 hover 表面与 text 主文字色；其尺寸、聚焦和禁用行为由同一共享控件维护。

### Inputs / Fields

通过 [settings/public/controls.ts](src/settings/public/controls.ts) 使用 Input、TextArea、Select、Toggle 等既有控件。输入聚焦以强调色边框与光环（3px、32% 强调色混色）反馈；Select 是按钮加门户菜单，不是原生 select，选项默认高度（28px），菜单圆角使用 menu。

Input、TextArea（含 plain）与聊天 composer 的占位提示使用 text-muted，保留为可读的次要文字；Input / TextArea 显式保持完整占位不透明度（1）。这项角色由共享输入样式与 composer 的既有样式负责人实现，不在局部页面重写；它不改变其他 text-faint 信息的角色。

Toggle 是紧凑胶囊开关（30×18px），滑块（14px）；开关状态使用交互强调色。不要替换为另一套局部基础控件。

### Chips / Tabs

可移除技术芯片使用等宽字体、胶囊轮廓与小型删除按钮；状态标签使用较小标签字与语义状态颜色，两者不是同一组件。分段切换在浅色轨道上抬升选中片段；深色状态使用 active 表面，保留其现有作用域规则。

### Cards / Containers

活跃的嵌入设置分组使用 settings-card 条目的提取规格、细边框与轻阴影；独立设置分组主要用分隔线，不套同样卡片。内嵌信息面板使用 muted 或 accent-soft 表面，现有圆角（7px，嵌入时 10px）。settings-controls.css 的同名基础样式目前无 TSX 使用，不作为已使用组件的证据。

### Navigation

聊天导航默认透明，悬停与选中使用文字色的薄洗色（4% / 6%）；对话选中洗色（7%）。嵌入设置导航沿用洗色，独立设置导航使用 hover / active 表面与强调图标。这是现状差异，不把其中一种追认为全仓规则。分组的手动顺序与对话钉住语义继续遵循既有领域规则。

### Dialogs / Focus

弹窗复用 kv-modal；基础圆角使用 menu、内边距使用 inset-card、宽度上限（340px），应用确认对话可为（360px）。保留 backdrop、焦点与 Escape 行为，不为单页重造弹窗。工具提示主要依赖原生 title；没有可提取的自定义 tooltip 皮肤。

聊天外壳和原生 dialog 中，按钮键盘聚焦光环为（2px、浅色 14% / 深色 18% 强调色混色）；输入光环不受指针来源隐藏规则限制。独立设置按钮不在上述选择器覆盖范围，保留原生焦点表现。不要声称各宿主已完全一致。

## Do's and Don'ts

### Do:
- Do 复用 Button / IconButton、settings/public/controls、kv-modal 与 custom-scrollbar；共享能力缺口在原负责人中补齐。
- Do 使用语义主题变量，并区分默认中性色板、启动兜底与当前用户主题。
- Do 按宿主容器处理空间变化，保留键盘焦点、菜单焦点恢复与减少动态效果行为。
- Do 保留平台差异：透明画布、原生材质、标题栏和字体渲染由已有实现负责。

### Don't:
- Don’t 用页面 CSS 覆盖共享控件的尺寸、颜色、边框、圆角和交互状态。
- Don’t 将官网的 Geist / Silkscreen 字体与营销视觉规则搬进桌面客户端。
- Don’t 固定白底蓝色、全局替换图片或品牌图标颜色，或把 --color-primary 当作 --accent。
- Don’t 在局部滚动区域添加 scrollbar-width / scrollbar-color，覆盖全局胶囊滚动条。
- Don’t 将未使用的 settings-card 样式或不存在的自定义 tooltip 当成已经交付的标准组件。

提取证据：src/index.css、src/theme/theme.ts、src/styles/shared-surfaces.css、src/styles/settings-01-embedded.css、src/styles/settings-02-pages.css、src/styles/chat-01-main.css、src/chat/persistence.ts。浏览器聊天首屏确认了系统字体、默认表面、侧栏宽度与空态标题；未验证 Tauri 原生行为或全功能界面。
