# 实现照片场景文字重绘模式

## Goal

面向照片和海报评估局部场景文字风格提取、长度适配与融合，不影响默认 UI 路径。

## Requirements

- P1：照片/海报模式与 UI/文档模式隔离，失败不得影响默认替换翻译。
- P2：评估 SRNet/MOSTEL 类“前景文字生成 + 背景修复 + 融合”架构及可部署模型，不直接假设论文模型满足产品约束。
- P3：支持源/目标文字长度变化、旋转、透视、颜色和基础风格匹配，并保留目标译文完整性。
- P4：推理只修改明确目标区域；区域外逐像素不变。
- P5：若本地模型在体积、延迟、字体覆盖或准确性上不达标，保留普通系统字体重绘作为降级。
- P6：第一版完成当前技术条件下可稳定落地的照片/海报重绘；不设置长期实验开关，但模型方案必须先通过最低门禁才可进入产品路径。
- P7：照片本地重绘目标不超过 3 秒；全部新增模型必须服从 Standard 250 MB / High 450 MB 总下载上限。

## Acceptance Criteria

- [ ] AC-P1：照片和海报样例的文字与背景融合无明显矩形贴片、旧字阴影或严重拉伸。
- [ ] AC-P2：中英跨语言和显著长短变化样例可读，译文不出现漏字、重复字或不可辨认字符。
- [ ] AC-P3：冷/热延迟、峰值内存和安装体积达到设计预算后才允许默认启用。
- [ ] AC-P4：模型失败时安全回退，不损坏 geometry-core、UI 和文档路径。
- [ ] AC-P5：达标模型随新场景路由直接启用；未达标时该场景由新 geometry-core 普通字体重绘处理，且不得保留旧管线或隐藏开关。

## Notes

- Keep `prd.md` focused on requirements, constraints, and acceptance criteria.
- Lightweight tasks can remain PRD-only.
- For complex tasks, add `design.md` for technical design and `implement.md` for execution planning before `task.py start`.
