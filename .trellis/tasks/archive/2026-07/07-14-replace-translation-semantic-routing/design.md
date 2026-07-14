# 跨平台语义几何与场景路由设计

## 1. Boundary

本任务提供跨平台 `SemanticGeometryProvider` 和场景分类，不负责最终翻译排版或像素修复。依赖 geometry-core 的统一 leaf/slot 契约。

## 2. Platform Abstraction

```rust
trait SemanticGeometryProvider {
    fn elements_in_capture(&self, capture: ScreenRect) -> Result<Vec<SemanticElement>, SemanticError>;
}

struct SemanticElement {
    id: String,
    role: SemanticRole,
    text: Option<String>,
    bounds_screen: ScreenRect,
    bounds_capture: Bounds,
    parent_id: Option<String>,
    children: Vec<String>,
    confidence: f32,
    source: SemanticSource,
}
```

- macOS adapter：AXUIElement/NSAccessibility，读取 role、title/value、children、position、size。
- Windows adapter：UI Automation，读取 ControlType、Name/Value、BoundingRectangle、parent/children。
- 两端在 Rust 边界归一化为 capture-relative physical-pixel coordinates。
- 查询必须有短超时；窗口消失、权限不足、跨进程失败都返回结构化错误并继续 OCR。

## 3. Geometry Fusion

- 先获取 OCR leaves，再将语义元素与 leaves 按文本相似度、覆盖率和中心距离匹配。
- 原生语义边界只在置信度达到阈值时覆盖 render slot；erase mask 仍以视觉文字 leaf 为准。
- 原生元素可提供列表行、按钮、文本框、父子容器等硬边界，防止跨控件合并。

## 4. Scene Router

输出 `UiList | UiControl | DocumentParagraph | Table | PhotoText | Unknown`、置信度和 reasons。

- UI：原生语义强信号、重复图标+文本行、控件边框。
- 文档：文本密度、列结构、标题/段落布局模型。
- 表格：单元格/分隔线或布局模型表格类别。
- 照片：纹理/边缘复杂、无稳定控件或文档结构。
- 低置信度统一 Unknown → exact_line。

## 5. Layout Model

先评估轻量文档布局模型（参考 PP-DocLayout-S 量级）；模型只参与 document/table 路由。若 ONNX 转换、跨平台推理、准确率或预算不达标，则使用现有分隔线+保守规则，不阻塞 UI 原生语义路径。

## 6. Cross-Platform Constraints

- Windows DPI virtualization、per-monitor scaling 与 macOS Retina 坐标必须用共享 fixture 验证。
- 多显示器负坐标、不同缩放和窗口移动期间均需覆盖。
- 两端必须同期通过门禁，不能以平台特例跳过核心验收。
