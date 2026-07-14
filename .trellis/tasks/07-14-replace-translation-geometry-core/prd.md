# 重构替换翻译几何与渲染契约

## Goal

拆分 OCR leaf、translation group、render slot 与 erase mask，恢复保守精确定位默认路径。

## Requirements

- G1：定义独立的 `OcrLeaf`、`TranslationGroup`、`RenderSlot` 和 `EraseMask` 契约，禁止一个区域同时拥有四种职责。
- G2：模型可按语义组获取上下文，但返回结果必须通过稳定 ID 映射到组；绘制器只消费 render slots。
- G3：默认保守模式保持 OCR 行/元素的原始顶部、左边界、基线、旋转和阅读顺序。
- G4：段落流排版必须顶部锚定；表格单元格、短标签和标题可按显式策略居中。
- G5：无法容纳完整译文时按“结构内扩展 → 换行 → 缩放”降级，不得越区、截断或改变首行锚点。
- G6：旧 `lens-replace-stream` 在迁移期提供兼容解码或一次性版本化升级，不允许前后端私自解释同一字段。
- G7：采用一次性新协议切换；只允许实现期短暂编译兼容，不保留运行时双管线、产品开关或旧激进聚合回退。

## Acceptance Criteria

- [ ] AC-G1：菜单/列表夹具的每一行绘制锚点相对原 OCR slot 的误差在约定阈值内，错误语义聚合不会移动第一行。
- [ ] AC-G2：同一段落可作为一个翻译上下文，但 render slots 数量和几何保持可独立验证。
- [ ] AC-G3：表格单元格不跨列，普通列表不合并为整页大段落。
- [ ] AC-G4：完整译文不被省略，且任何 slot 不覆盖无关图标或相邻结构。
- [ ] AC-G5：旧事件载荷迁移测试、ID 乱序/缺失回退测试和 Canvas 自然尺寸测试通过。
- [ ] AC-G6：新管线通过门禁后，旧区域聚合和旧 Canvas 绘制路径被删除，生产代码只有一个行为来源。

## Notes

- Keep `prd.md` focused on requirements, constraints, and acceptance criteria.
- Lightweight tasks can remain PRD-only.
- For complex tasks, add `design.md` for technical design and `implement.md` for execution planning before `task.py start`.
