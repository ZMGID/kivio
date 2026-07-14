# 接入语义几何来源与场景路由

## Goal

Windows UI Automation 与 macOS Accessibility 同期接入，结合文档布局检测和保守 OCR 回退，统一识别 UI、段落、表格与照片场景。

## Requirements

- S1：建立场景路由，至少区分 UI/菜单、文档段落、表格/网格、照片文字和 unknown。
- S2：macOS 读取 Accessibility 元素的角色、值/标题、父子层级、屏幕位置和尺寸；Windows 通过 UI Automation 获取对等语义与几何。
- S3：两端原生 API 适配为共享 `SemanticElement`/`SemanticGeometryProvider` 契约，统一坐标系、置信度、来源、超时和错误语义。
- S4：Accessibility/UI Automation 缺失、超时或结果不可信时无感回退 OCR，不得阻塞替换翻译。
- S5：评估轻量文档布局模型用于标题、文本、列表、表格和多栏；不得用文档模型强行解释任意 UI。
- S6：unknown 默认使用 geometry-core 的精确行级回退。
- S7：场景判定必须输出原因和置信度，供调试夹具和发布门禁复现。
- S8：UI/文档语义几何与布局推理热路径计入 1 秒本地预算；新增模型计入 Standard 250 MB / High 450 MB 总包预算。

## Acceptance Criteria

- [ ] AC-S1：macOS 原生应用/可访问网页和 Windows 原生应用/浏览器中的按钮、列表项、文本框能使用真实 UI 边界。
- [ ] AC-S2：同一共享 fixture 在 Windows 与 macOS 产出等价的标准化语义角色、父子关系和捕获区相对坐标。
- [ ] AC-S3：Canvas、自绘控件、图片和无辅助功能应用自动回退 OCR，结果不比精确行级基线差。
- [ ] AC-S4：表格、多栏文档和普通 UI 不互相误路由；低置信度统一进入 unknown。
- [ ] AC-S5：Accessibility/UI Automation 查询均具备超时、权限缺失、窗口消失、DPI/缩放和多显示器坐标测试。

## Notes

- Keep `prd.md` focused on requirements, constraints, and acceptance criteria.
- Lightweight tasks can remain PRD-only.
- For complex tasks, add `design.md` for technical design and `implement.md` for execution planning before `task.py start`.
