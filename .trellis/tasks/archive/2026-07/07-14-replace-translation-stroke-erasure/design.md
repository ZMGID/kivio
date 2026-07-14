# 笔画级原文擦除设计

## 1. Boundary

本任务生成可靠 `EraseMask` 和 cleaned image。它消费 geometry-core 的 OCR leaves，但不使用 translation groups 或 render slot 大矩形作为 mask。

## 2. Mask Pipeline

```text
OCR leaf polygon
  → stroke probability / deterministic foreground estimate
  → polygon-constrained refinement
  → antialias/shadow-aware dilation
  → icon/rule protection
  → union mask
```

- 优先评估轻量文字笔画分割模型，输入局部 patch 并输出 stroke probability。
- 模型不可用时使用现有颜色簇回退，但背景选择、阈值和 tie-break 必须确定性。
- OCR 欠框只沿文字主方向有限扩张；垂直方向不得吃掉分隔线。
- 头像、按钮图标、表格线和控件边界通过 semantic/layout protection masks 排除。

## 3. Repair Router

- Flat UI：多方向最近背景采样 + 局部结构传播。
- Table/code badge：在保护 mask 约束下传播，禁止跨边界取色。
- Gradient/light texture：局部结构修复或轻量 inpainting。
- Photo：MI-GAN；只在 union stroke mask 内合成。

## 4. Model Evaluation

候选笔画模型按以下门禁比较：多语言、旋转/描边/阴影、UI 图标误报、ONNX 可转换、CPU 延迟、文件大小、商业许可。未达标时不进入产品依赖。

## 5. Invariants

- mask 外像素逐像素不变。
- 同输入重复运行输出一致。
- 修复失败返回结构化 warning，并使用新确定性快速路径。
- 不允许用单色大矩形或模糊块作为照片级擦除成功结果。
