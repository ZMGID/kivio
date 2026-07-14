# 升级笔画级原文擦除流水线

## Goal

以稳定的文字笔画分割替代颜色阈值主导的 mask，并保留 UI 结构与照片修复分流。

## Requirements

- E1：擦除 mask 以文字笔画为目标，不以 OCR 行矩形或语义组矩形为目标。
- E2：评估可本地部署的轻量文字笔画分割模型；现有颜色聚类仅作为确定性回退。
- E3：UI/表格快速路径保护边框、图标、控件填色和局部渐变；照片路径使用 MI-GAN 或后续修复器。
- E4：mask 外像素保持逐像素不变；任何生成模型结果只合成到 mask 内。
- E5：OCR 欠框、抗锯齿、描边、阴影和旋转文字具有明确的膨胀/细化策略。
- E6：背景簇、mask 和修复结果必须确定性可复现，不受哈希迭代等随机顺序影响。
- E7：笔画分割与 UI/文档修复计入 1 秒热路径预算，模型文件计入统一离线包上限。

## Acceptance Criteria

- [ ] AC-E1：纯色 UI、代码徽章、表格、渐变和照片夹具无明显原文字形残影。
- [ ] AC-E2：图标、头像、分隔线和单元格边界不被擦除或生成模型重建。
- [ ] AC-E3：mask 外像素一致性测试覆盖全部修复路径。
- [ ] AC-E4：相同输入重复运行产生一致 mask 和确定性快速修复结果。
- [ ] AC-E5：性能与模型体积预算在设计阶段固化，并在真实 Apple M 系列设备验证。

## Notes

- Keep `prd.md` focused on requirements, constraints, and acceptance criteria.
- Lightweight tasks can remain PRD-only.
- For complex tasks, add `design.md` for technical design and `implement.md` for execution planning before `task.py start`.
