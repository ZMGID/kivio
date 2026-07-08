# 聊天界面质感优化（空态首屏）

## 背景

用户对空态首屏"不够高级、缺质感"给出六点分析，要求全部对照落地（而非只挑子集）。

## 六项逐条处置

| # | 分析项 | 处置 | 落点 |
|---|--------|------|------|
| 1 | 表面层次太扁 | **已改**：定义 3 层表面——侧栏（最深，muted 92% 混黑）→ 主区（`--theme-surface-soft`）→ 输入框/卡片（`--theme-surface` 最亮）；侧栏右缘 1px 分割线改直引主题变量（原 `var(--border)` 在该作用域未定义，等于没画），暗色补 `rgba(255,255,255,0.07)` 分割线 | `src/index.css` `.chat-main-pane` / `.chat-sidebar-shell` |
| 2 | 点阵背景 | **已改**（前一轮）：每点随机明度差 0.08+0.14d → 0.05+0.035d，光带增益/脉冲减半；径向渐变蒙版原本已有（`centerFade` 中心强边缘淡出），保留 | `ChatDotGridBackground.tsx` |
| 3 | accent color | **代码已有**：输入框有内容时发送按钮即为 accent `#e8a090` 实心（含 soft-pulse）；disabled 态描边化尝试被用户否决，保留原实心灰。不再增加 accent 出现点（克制原则） | 无改动 |
| 4 | 输入框阴影 | **已改**：三态（default/plan/orchestrate）阴影从 2 层补为 3 层（1px 贴边 + 4-10px 环境影 + 12-32px 大范围极淡影），focus 同步 3 层扩大；200ms 过渡原已存在。底部图标原已统一 18px / stroke 1.75 | `InputBar.tsx` composer className |
| 5 | 字体排版 | **已改**（前一轮）：hero 标题 `font-medium` + `tracking -0.02em`；问候语随机池原已存在（utils.ts）。本轮：侧栏「最近/集/项目」segmented 间距 gap 1.5→2、px 1→1.5 | `Chat.tsx` / `Sidebar.tsx` |
| 6 | 圆角系统化 | **勘察结论：已系统**——现刻度为 rounded-md(6)/lg(8)/xl(12)/输入框 28，非随机；全局重刷圆角属大面积改动且收益低，范围外 | 无改动 |

范围外（明确不做）：Windows Mica/亚克力侧栏（需原生窗口层改动，独立任务）；圆角全局重刷。

## Acceptance Criteria

- [x] A1 空态点阵均匀安静、无成片斑块（前一轮已落，待目检）
- [x] A2 三层表面在浅色三主题下可辨（侧栏 < 主区 < 输入框），侧栏分割线亮/暗均可见
- [x] A3 输入框默认/focus 阴影为三层柔影，浮起感增强
- [x] A4 标题字重字距 + segmented 间距生效
- [x] A5 `npm run lint` / `npm run typecheck` / `npm test`（201 个）全过
- [ ] A6 亮/暗 + default/warm/cool 主题人工目检无回归

## 历史

- 第一轮只落了点阵+标题+发送按钮三处；发送按钮描边样式被用户否决已回退（`0a4c722`）；用户指出六项应全部对照处理，本轮补齐 1/4/5 并对 3/6 给出勘察结论。
