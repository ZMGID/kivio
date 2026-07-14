# 建立替换翻译视觉基准与发布门禁

## Goal

建立 UI、列表、表格、文档、照片夹具和几何、残影、结构保护、完整性指标。

## Requirements

- V1：建立版本化夹具集：菜单/列表、按钮/输入框、普通段落、多栏、表格、代码块、渐变、自然照片、旋转文字和极长译文。
- V2：记录原图、OCR leaves、translation groups、render slots、mask、cleaned image 和最终重绘结果。
- V3：定义几何误差、区域重叠、原文残影、结构像素保护、译文完整性、OCR/翻译局部回退和性能指标。
- V4：每个已知回归截图必须转成可重复 fixture 或等价合成测试，禁止只保存聊天截图结论。
- V5：发布门禁按场景分别统计，不能用平均分掩盖某类截图完全不可用。
- V6：Windows 与 macOS 使用同一逻辑 fixture 清单，并增加平台特有的 DPI、Retina、缩放、多显示器和原生语义 API 用例。
- V7：照片原风格重绘单独统计首版门禁，不得用 UI/文档路径的通过率为其背书；未通过时阻止对应模型进入产品路径。

## Acceptance Criteria

- [ ] AC-V1：当前用户提供的文档列表、仓库菜单、表格和照片样例均有可重复验证记录。
- [ ] AC-V2：任何新布局或 mask 修改都能输出 before/after 和量化差异。
- [ ] AC-V3：几何错位、跨列、图标误擦、段落整体下移和原文残影均有独立回归测试。
- [ ] AC-V4：CI/本地质量命令可明确阻止未达到门禁的变更进入发布。
- [ ] AC-V5：Windows 与 macOS 都通过核心几何、语义路由、擦除和最终重绘门禁后，相关子任务才可归档。

## Notes

- Keep `prd.md` focused on requirements, constraints, and acceptance criteria.
- Lightweight tasks can remain PRD-only.
- For complex tasks, add `design.md` for technical design and `implement.md` for execution planning before `task.py start`.
