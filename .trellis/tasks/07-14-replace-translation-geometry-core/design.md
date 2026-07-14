# 替换翻译几何与渲染契约设计

## 1. Boundary

本任务只解决核心数据模型、翻译映射和 Canvas 排版，不接入平台语义 API、新笔画模型或照片风格模型。它是其他四个子任务的前置依赖。

## 2. Core Contracts

```text
OcrLeaf[]
  ├─> EraseMaskSource[]     仅描述待擦字形
  ├─> TranslationGroup[]    仅描述翻译上下文
  └─> RenderSlot[]          仅描述绘制锚点与结构边界
```

建议契约：

```rust
struct OcrLeaf {
    id: String,
    text: String,
    polygon: [Point; 4],
    bounds: Bounds,
    confidence: Option<f32>,
    baseline: Option<Baseline>,
    source: GeometrySource,
}

struct TranslationGroup {
    id: String,
    leaf_ids: Vec<String>,
    source_text: String,
    scene_kind: SceneKind,
}

struct RenderSlot {
    id: String,
    group_id: String,
    leaf_ids: Vec<String>,
    bounds: Bounds,
    anchor: TextAnchor,
    flow: RenderFlow,
    style: SourceTextStyle,
}
```

前端最终事件携带 `groups`、`slots` 和 `cleanedImage`。翻译结果仍按 group ID 映射；布局器把完整译文确定性分配到 slots。

## 3. Layout Rules

- `exact_line`：菜单、列表、按钮、短标签和 unknown 默认一条视觉行对应一个翻译组和一个 slot；模型仍一次接收全部组作为页面上下文，但译文不得跨行重新分配。
- `paragraph_flow`：仅在后续语义路由明确证明段落成员关系时启用；OCR 间距/缩进启发式本身无权创建跨行翻译组。
- `cell_flow`：每个表格单元格独立，禁止跨 cell。
- `scene_patch`：照片文字使用旋转局部 patch，由后续 scene task 消费。
- 完整译文依次尝试：原字号附近换行、结构内可用空间、7px 下限、安全等比缩放。
- `exact_line` 的正常与安全缩放路径都从 OCR `anchor.x/anchor.y` 绘制，不得根据译文高度垂直居中。

## 4. Event Migration

- 一次性把 `LensReplaceRegion[]` 升级为版本化 `ReplaceRenderPayloadV2`。
- Lens 事件边界集中解码，组件不读取未类型化字段。
- 开发期间允许编译兼容适配器，但生产路径不保留 V1/V2 双分支。

## 5. Rollback

提交按“契约 → 后端生成 → 前端解码 → Canvas 渲染 → 删除旧路径”分段。任一步失败可回到上一个编译通过点；不得恢复旧激进段落聚合作为产品回退。
