# 替换翻译 V2 总体设计

## 1. Architecture Goal

生产环境只保留一条替换翻译管线。核心原则是：翻译上下文、绘制几何和擦除几何彼此独立，任何语义合并都不能改变原文首行锚点或扩大擦除范围。

```text
CapturedFrame
  ├─ OCR leaves
  ├─ macOS Accessibility / Windows UI Automation
  └─ document-layout hints
          ↓ normalize
SemanticElement + OcrLeaf
          ↓ scene router
UI | Document | Table | Photo | Unknown
          ↓ geometry core
TranslationGroup + RenderSlot + EraseMask
          ├─ cloud translation by stable group ID
          └─ local stroke erasure / repair
                    ↓
cleaned image + complete translations
                    ↓
natural-size deterministic renderer
```

## 2. Core Contracts

### 2.1 OcrLeaf

保存 OCR 的最小可定位单元：稳定 ID、原文、四点多边形、轴对齐 bounds、基线、旋转、阅读顺序、置信度和来源。它是其他阶段的证据，不直接代表翻译组、绘制槽或擦除 mask。

### 2.2 TranslationGroup

只负责提供翻译上下文和稳定 ID。一个 group 可引用多个 leaves，但不得拥有最终绘制矩形。翻译响应按 ID 回填，缺失或非法项只回退本组原文。

### 2.3 RenderSlot

只描述译文的可绘制空间、首行/顶部锚点、基线、旋转、对齐、层级、结构边界和来源。一个 translation group 可以映射到多个 slots；菜单、列表和 unknown 默认保持叶子或视觉行粒度。

### 2.4 EraseMask

只描述需要移除的文字笔画像素。mask 从 OCR/语义几何和笔画分割生成，不允许使用 translation group 或 render slot 的大矩形代替。

## 3. Geometry and Coordinate System

- 后端和前端统一使用捕获图像自然像素坐标。
- `CapturedFrame` 明确记录屏幕逻辑坐标、物理像素、显示器 scale factor、裁剪偏移和安全工作区。
- macOS Retina、Windows DPI、多显示器与负坐标在平台适配层归一化。
- Canvas backing size 等于 cleaned image 的 natural size，CSS 只负责显示缩放。
- 段落顶部锚定；行、短标签、标题和表格单元格只按显式 slot 策略布局。

## 4. Semantic Geometry Providers

共享接口 `SemanticGeometryProvider` 返回标准化 `SemanticElement`：角色、文本、父子层级、屏幕 bounds、捕获区相对 bounds、来源、置信度和诊断原因。

- macOS provider：Accessibility API。
- Windows provider：UI Automation。
- 查询必须有超时并处理权限缺失、窗口消失、不可访问 Canvas、自绘控件和多显示器坐标。
- 平台结果不可信时无感回退 OCR，不阻塞整个任务。

## 5. Scene Routing

路由输出 `sceneKind`、置信度、原因和使用的证据：

- UI/menu/list：优先原生语义元素边界，逐元素或逐行绘制。
- Document paragraph：允许 translation group 合并上下文，render slots 仍保留结构和顶部锚点。
- Table/grid：单元格边界是硬约束，不跨列、不覆盖表格线。
- Photo text：使用笔画擦除和通过门禁的照片重绘器。
- Unknown：保守精确行级绘制。

文档布局模型只是候选证据，不得强行解释任意 UI。

## 6. Erasure and Repair

1. OCR polygon/semantic bounds 提供文字候选区域。
2. 轻量笔画分割为主，确定性颜色/对比度方法为回退。
3. 根据字号、描边、阴影和旋转做受限膨胀与细化。
4. 保护高亮、图标、头像、边框、分隔线和表格线。
5. UI/表格走确定性快速修复；复杂照片可走 MI-GAN 或后续通过门禁的修复器。
6. 所有生成结果只在 mask 内合成，mask 外逐像素不变。

## 7. Rendering

- 翻译使用稳定 ID JSON 契约，不依赖数组顺序。
- 完整文本按“结构内扩展 → CJK/Latin 换行 → 二分字号 → 安全等比缩放”适配。
- 不摘要、不截断、不省略，不以 Canvas `maxWidth` 横向压扁文字。
- 颜色从文字/背景证据估计，置信度不足时按 cleaned background 对比度选择。
- UI/文档默认使用系统字体；照片候选采用背景修复、确定性目标字形条件、风格编码与仿射融合。

## 8. Photo Model Gate

照片模型必须单独通过：中文/英文覆盖、长短变化、旋转/透视、旧字残影、区域外一致性、热路径 3 秒、峰值内存、许可证和总包体门禁。未通过时 photo route 仍使用 V2 geometry-core 系统字体渲染，不保留旧管线或隐藏开关。

## 9. Resource and Runtime Boundaries

- Standard 总离线下载不超过 250 MB；High 不超过 450 MB。
- UI/文档本地热路径不超过 1 秒；照片本地路径不超过 3 秒，均不含云端翻译。
- OCR、笔画分割、修复和照片候选共享离线 manifest、校验、原子安装和可观测进度。
- 本地修复与云端翻译在几何确定后并行；只有最终 cleaned image 与完整 slots 就绪后才展示覆盖层。

## 10. Protocol and Migration

- `lens-replace-stream` 一次性升级为版本化 V2 载荷，明确 leaves/groups/slots、cleaned image 和诊断字段。
- 实施期间可以有短暂的类型兼容，但生产运行时不能保留新旧行为切换。
- 新管线达到视觉门禁后删除旧 `ReplaceLayoutRegion` 多职责聚合、矩形覆盖和旧 Canvas 绘制路径。
- 普通截图翻译、选中文本翻译和知识库 RapidOCR 不在协议迁移范围内。

## 11. Task Boundaries

- `geometry-core`：四类契约、坐标、稳定 ID、slot 适配和自然尺寸渲染。
- `semantic-routing`：macOS Accessibility、Windows UI Automation、文档证据和场景路由。
- `stroke-erasure`：笔画 mask、结构保护、确定性/照片修复分流。
- `scene-rendering`：照片风格候选评估和达标接入。
- `visual-benchmark`：夹具、指标、分桶门禁和跨平台报告。

## 12. Rollback and Failure Policy

- 单个语义 provider、模型或翻译组失败时回退 V2 的保守 OCR/系统字体路径，不能切回旧架构。
- 视觉门禁未通过的照片模型不进入生产包。
- 任何坐标或结构置信度不足时减少合并范围；优先避免错误覆盖，而不是猜测更大的语义区域。
