# Design: 显式文件展示通道

## Core Design

采用“原生展示工具 + 现有工具 segment 作为结构化位置锚点”。不新增 Markdown 占位符，也不依赖文件路径。

```text
write / run_python / MCP result
  -> ChatToolArtifact{id}
  -> tool result 向模型列出 artifact ID
  -> 模型调用 present_artifacts(ids, caption)
  -> ToolCallRecord.structured_content(type=artifact_presentation)
  -> 该 tool segment 在时间线原位渲染 ArtifactPresentationBlock
```

## Data Contract

`ChatToolArtifact` 新增可选 `id`：

```text
id?: string
```

可选是为了兼容旧消息。新工具结果在 agent executor 中统一补齐 `art_<uuid>`。

`present_artifacts` 参数：

```json
{
  "artifact_ids": ["art_xxx"],
  "caption": "可选说明"
}
```

工具返回结构化内容：

```json
{
  "type": "artifact_presentation",
  "artifactIds": ["art_xxx"],
  "caption": "可选说明"
}
```

展示引用只在当前 `ChatMessage` 聚合出的 artifact map 中解析。

## Rendering

- `present_artifacts` 识别为独立时间线节点，不折叠进普通工具组。
- 该工具卡不显示常规 `ToolCallBlock`，改为展示引用的图片和文件卡。
- 图片复用现有图片画廊组件；文件复用单文件卡组件。
- 无效引用显示“文件不可用”，绝不回退加载路径。
- 页面末尾自动区域只处理无 `id` 的历史 artifact；带 `id` 的新 artifact 必须显式展示。

## Compatibility

- 历史 artifact 没有 `id`，继续自动显示。
- 历史 Markdown 图片名匹配继续读取全量 artifact。
- 新字段均可选，不需要存储迁移。
- 直接生图的消息级 artifact 保持现有正文 Markdown/画廊行为；本任务主要改变工具级 artifact。

## Security Boundary

- 展示工具 schema 不存在 path/url 字段。
- 前端只按 artifact ID 查当前消息内存对象。
- 未解析 ID 只显示错误占位，不尝试把 ID 当 URL、文件名或路径。

## Main Files

- `src-tauri/src/mcp/types.rs`
- `src-tauri/src/mcp/native_registry.rs`
- `src-tauri/src/chat/agent/execute.rs`
- `src-tauri/src/chat/agent/prepare.rs`
- `src/chat/types.ts`
- `src/chat/segments.ts`
- `src/chat/MessageBubble.tsx`
- `src/chat/GeneratedFileArtifacts.tsx`
