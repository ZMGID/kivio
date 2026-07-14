# 照片场景文字重绘设计

## 1. Boundary

仅处理 scene router 判定为 `PhotoText` 的区域。UI、文档、表格和 Unknown 不依赖本任务模型。

## 2. Candidate Architecture

参考场景文字编辑研究，将任务拆成：

```text
local rotated patch
  ├─> background inpainting
  ├─> source style encoding + target glyph rendering
  └─> affine placement + alpha fusion
```

- 背景沿用 stroke-erasure cleaned patch。
- 目标文字内容由确定性字体渲染器生成 glyph condition，禁止生成模型自由拼写译文。
- style encoder 只负责颜色、粗细、倾斜、基础纹理等外观，不负责文本内容正确性。
- affine fusion 明确控制旋转、尺度、边距和目标区域，适配源/目标长度变化。

## 3. Candidate Gate

候选包括可合法使用并能本地部署的 SRNet/MOSTEL 类模型或更轻的自研组合。任何方案必须满足：

- 中英字符可读且完整；
- 长短变化无严重拉伸、挤压、重复或漏字；
- Windows/macOS CPU/可用加速后端一致；
- 照片本地处理不超过 3 秒；
- 总离线包符合 Standard 250 MB / High 450 MB；
- 许可证可用于产品分发。

## 4. Product Behavior

- 达标模型直接用于 PhotoText 路由，不增加长期实验开关。
- 未达标时 PhotoText 使用 geometry-core 的旋转 slot + 普通字体完整重绘。
- 模型失败时单区域降级，不使整张替换翻译失败。

## 5. Non-goals

- 不承诺复刻任意稀有字体、3D 特效、复杂发光/阴影。
- 不使用扩散模型自由生成目标文字作为首版主路径。
