# Kivio 视觉风格指南（Kivio Design Language）

> 适用范围：Kivio 官网及所有对外品牌页面（未来的产品页、发布页、剥离出去的子产品页均沿用此风格）。
> 来源：v1 版官网（`website/index.html`），确立 Kivio 自己的极简工程美学。
> 本文档是权威来源 —— 新页面直接按此实现，不要凭记忆复刻。

---

## 1. 风格一句话

**白纸上的像素信号。**

纯白画布 + 黑白灰三阶文字 + 像素抖动（dithering）动效 + 像素标题字体。没有彩色、没有渐变、没有阴影发光 —— 唯一的"装饰"是一片会流动的黑色像素场，像旧打印机 / CRT 信号一样克制而有机。工程文档的口吻，杂志排版的留白。

---

## 2. 色彩

整站只允许下列颜色。**禁止**引入任何品牌蓝、渐变、彩色 hover。

| Token | 值 | 用途 |
|---|---|---|
| `--bg` | `#ffffff` | 页面背景（唯一背景色） |
| `--fg` | `#0a0a0a` | 主文字、实心按钮底、eyebrow 方块 |
| `--muted` | `#737373` | 次级文字、正文段落 |
| `--faint` | `#a3a3a3` | 弱化文字（编号、meta 行、SCROLL 提示） |
| `--border` | `#e5e5e5` | 所有分隔线、卡片描边、网格缝隙 |
| `--dark-bg` | `#050608` | 收尾深色区（全站唯一深色块）背景 |
| hover 背景 | `#fafafa` | 卡片 hover、代码窗格底 |
| 成功绿 | `#16a34a` | 仅限模拟运行轨迹里的 `✓` 行，别处不用 |

深色区内文字：白色按透明度分层 —— 标题 `#fff`、正文 `rgba(255,255,255,0.6)`、链接 `0.85`、页脚 `0.45`、分隔线 `rgba(255,255,255,0.12)`。

选区样式：`::selection { background: #0a0a0a; color: #fff; }`（黑底白字，反转）。

---

## 3. 字体（关键识别元素）

三个字体族，职责严格分离：

```css
--sans:  'Geist', 'Noto Sans SC', -apple-system, 'PingFang SC', 'Microsoft YaHei', sans-serif;
--mono:  'Geist Mono', ui-monospace, SFMono-Regular, Menlo, monospace;
--pixel: 'Silkscreen', 'Geist Mono', monospace;
```

Google Fonts 引入：`Geist:wght@300;400;500;600;700`、`Geist+Mono:wght@400;500`、`Silkscreen:wght@400;700`、`Noto+Sans+SC:wght@300;400;500;700`。

### 3.1 像素字体 `--pixel`（Silkscreen Bold 700）

**这是品牌最强的识别符号**，用户明确锁定的两处用法：

- **Hero 大标题 "KIVIO"**：`font-size: clamp(3.2rem, 11vw, 8.5rem)`，`line-height: 0.94`，`letter-spacing: 0.02em`，全大写。
- **产品区标题 "Kivio Desktop"**：`clamp(2.4rem, 5.6vw, 4.6rem)`，`line-height: 0.98`。

规则：像素字体**只用于品牌名 / 产品名级别的大标题**（每屏最多一处），永远不用于正文、按钮、导航。它与像素抖动背景在视觉语法上是同一件事 —— "信号里长出来的字"。

### 3.2 无衬线 `--sans`（Geist / 思源黑体）

- 中文 tagline（"屏幕所至，智能所及。"）：**Light 300**，`clamp(1.5rem, 3.6vw, 2.4rem)`，中文加 `letter-spacing: 0.04em` —— 大号超细中文与像素黑标题形成刚柔对比，这个对比是 hero 的第二识别点。
- 章节标题：600，`letter-spacing: -0.02em`（中文改 `0.02em`，行高放宽到 1.25）。
- 正文：400，14–15px，`line-height: 1.8`，颜色 `--muted`。

### 3.3 等宽 `--mono`（Geist Mono）

所有"工程感"小字：导航 logo 字（`letter-spacing: 0.18em`）、eyebrow、meta 行（`V2.7.5 — MACOS · WINDOWS — GPL-3.0`）、卡片编号（01–12）、SCROLL 提示、模拟运行轨迹。统一小号（10.5–13px）+ 大写 + 宽字距（`0.08em–0.2em`）。

---

## 4. 像素抖动动效（Dithering Hero，核心资产）

用户最喜欢的元素。全屏 `<canvas>`，Bayer 4×4 有序抖动渲染一个流动波场，视觉上是右侧一片黑色像素云在缓慢呼吸流动。

### 4.1 算法要点

- 低分辨率渲染：canvas 内部尺寸 = 视口 / `CELL`（CELL = 3.4px），CSS 拉伸回全屏并加 `image-rendering: pixelated` —— 像素颗粒感来自这一步，**不要**在全分辨率下画小方块。
- 每个格子算一个亮度 `v`（两层正弦波干涉 + 到焦点的径向衰减），再与 Bayer 阈值矩阵比较得到纯黑/纯白 —— 没有灰度过渡，抖动纹理由此产生。
- 黑色像素值用 `26`（不是纯 0），白色 `255`。
- 帧率锁 ~20fps（`ts - last < 50` 跳帧）：抖动动画低帧率反而更有"信号感"，还省电。
- 焦点位置 `cx = W * 0.68, cy = H * 0.42`（右偏上），给左侧文字让位。

核心公式（完整实现见 `website/index.html` 内 `/* dithering hero canvas */` 段）：

```js
const CELL = 3.4
const BAYER = [[0,8,2,10],[12,4,14,6],[3,11,1,9],[15,7,13,5]]
// 每格：
const t = tms * 0.00028
let v = 0.5 + 0.5 * Math.sin(x * 0.055 + t * 2.2 + Math.sin(y * 0.045 - t * 1.4) * 2.4)
v *= 0.55 + 0.45 * Math.sin(dist * 9 - t * 3.2)   // 径向波纹
v *= Math.max(0, 1 - dist * 1.55)                 // 径向衰减，dist 以 (0.68W, 0.42H) 为中心
const on = v > (BAYER[y & 3][x & 3] + 0.5) / 16   // 有序抖动阈值
```

### 4.2 白色遮罩（让文字浮在信号上）

canvas 之上盖一层 `.hero-veil`，两个渐变叠加：以文字区（30%, 52%）为中心的径向白洞 + 顶底线性渐白。文字永远排在遮罩之上（`z-index: 2`）。

### 4.3 性能与可访问性（必须保留）

- `IntersectionObserver`：hero 滚出视口即 `cancelAnimationFrame`，回来再启动。
- `prefers-reduced-motion: reduce`：不跑动画，渲染固定单帧 `frame(4000)`（静止的抖动云依然有质感）。

### 4.4 逐字升起动画

KIVIO 五个字母和 tagline 逐字 `<span>` 拆分，`translateY(0.55em) → 0` + 淡入，缓动 `cubic-bezier(0.19, 1, 0.22, 1)`，字母间隔 55ms、tagline 26ms（延迟 300ms 起）。语言切换时 tagline 重新拆字、动画重放。

---

## 5. 版式与布局

- 容器：`max-width: 1120px; padding: 0 24px`。
- 页面结构（顺序固定）：固定导航 → 全屏 hero（`min-height: 100svh`）→ 产品区 → Agent Runtime 展示条 → 12 格能力网格 → 深色收尾区（含页脚）。
- 分区之间只用 `1px --border` 顶边线分隔，`padding: 110px 0`。**没有**卡片阴影、没有圆角大卡片 —— 层次全靠留白和 1px 线。
- 12 格网格：`grid-template-columns: repeat(4, 1fr); gap: 1px; background: var(--border)`（用 1px gap 露出底色当格线），单元格白底，hover 变 `#fafafa`，右上角 `+` hover 旋转 90°。响应式 4→2→1 列。
- 规格清单（spec-list）：`<dl>` 行式表格，左列 mono 大写小字标签（130px），右列正文，行间 1px 线 —— "参数表"是本风格叙述产品的标准方式，优先于营销段落。
- 圆角：按钮/胶囊 `border-radius: 999px`、logo 图 4–5px。**官网的**分区、12 格网格、spec-list 一律直角，靠 1px 线分隔。功能性小卡片（如产品内的信息卡 / 工具卡）可用 ≤6px 的小圆角（`rounded-md`）让边缘柔和，但**禁止大圆角容器**（≥12px）——大圆角会滑向"App 卡片"观感，偏离本风格。

## 6. 组件速查

- **实心按钮**：黑底白字胶囊，`padding: 11px 24px`，hover `opacity: 0.8` + 上移 1px。深色区内反转为白底黑字。
- **文字链接**：纯文字 + 尾缀符号（`↗` 外链 / `↓` 下载，mono 字体），hover 出现下边框。**不用**彩色链接。
- **Eyebrow**：mono 大写小字 + 前置 7×7px 黑色实心方块；可附加胶囊状态标签（如"持续构建中"）。
- **导航**：透明起步，滚动 >10px 后白色半透明 + `backdrop-filter: blur(14px)` + 底边线。
- **模拟运行轨迹窗格**：`#fafafa` 底，mono 13px/行高 2.05，行格式 `⚙ tool_name 参数`（工具名黑、参数灰），结果行绿 `✓`，末尾黑色方块光标闪烁（`steps(2)` 1.1s）。
- **滚动渐入**：`.rv` 类，`translateY(18px)` + 淡入 0.65s，IntersectionObserver 触发一次即停。
- **SCROLL 提示**：hero 底部居中，mono 小字 + 1px 竖线伸缩动画。

## 7. 文案口吻

- 短句、克制、工程文档味；避免感叹号和营销腔。
- 标题偏格言体："屏幕所至，智能所及。" / "探索屏幕，创造可能。" / "Your screen, intelligent."
- 能力描述压缩成一行名词句，句号结尾。
- 中英双语等价维护（`data-i18n` 字典），默认跟随浏览器语言，选择存 `localStorage('kivio-lang')`。

## 8. 禁止清单

1. 任何彩色/渐变/发光/玻璃拟态（深色收尾区除外，它也只是纯色）。
2. 像素字体用于正文、按钮、导航。
3. 卡片阴影、大圆角容器。
4. 抖动动效改成平滑灰度渐变（灵魂就在黑白二值抖动上）。
5. 提及已剥离的产品形态（独立终端产品 Kivio Code）—— 网站叙事聚焦 Agentic + GUI。
