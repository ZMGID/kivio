# 替换翻译视觉基准与发布门禁设计

## 1. Purpose

把“看起来是否更好”变成可重复输入、阶段产物和量化指标。本任务不修改产品算法，但为全部子任务提供第一批 fixture，并最终决定是否可发布。

## 2. Fixture Schema

每个 fixture 目录包含：

```text
source.png
case.json                 场景、语言、平台、缩放、预期结构
expected_geometry.json    leaves/groups/slots/anchors
expected_protection.png   不允许修改的像素区域
translations.json         固定完整译文与异常响应变体
```

运行后产物：OCR、semantic elements、scene route、groups、slots、mask、cleaned image、final image、metrics.json 和 contact sheet。

## 3. Dataset Matrix

- UI：菜单、仓库列表、按钮/输入框、卡片、头像+标签、代码徽章。
- Document：标题、普通段落、编号列表、多栏、代码块。
- Table：有线/无线、交替底色、长译文。
- Photo：纯色招牌、渐变、纹理、旋转/透视、阴影。
- Platform：macOS Retina/多显示器；Windows 100/125/150/200% DPI/多显示器。
- Failure：OCR 漏字/图标误识别、翻译 ID 乱序/缺失/重复、模型缺失/推理失败。

## 4. Metrics

- Geometry：slot top/left/baseline/rotation error，first-anchor drift。
- Structure：跨区域重叠率、跨列率、保护区域像素变化率。
- Erasure：mask recall proxy、旧字 OCR 回读、残影评分、mask 外一致性。
- Translation：完整性、ID 映射、目标 OCR CER、无截断。
- Visual：SSIM/LPIPS 仅作辅助；UI 结构指标优先。
- Performance：阶段耗时、峰值内存、下载/安装体积。

## 5. Gates

门禁按 UI、document、table、photo 和 platform 分桶；任何必需桶失败都阻止对应子任务完成。不得只看平均分。阈值先由当前精确行基线和用户失败样例校准，再在 implement 中固化。

## 6. Privacy

用户截图默认只作为本地 fixture；进入仓库前需确认无敏感信息并最小裁剪。不能提交的样例用等价合成 fixture 和本地路径 manifest 复现。
