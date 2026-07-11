# Implementation Plan

1. 扩展 artifact 数据契约，统一为新工具产物分配稳定 ID。
2. 新增并注册 `present_artifacts` 原生工具，返回结构化展示声明。
3. 在工具结果文本中提供可用 artifact ID，并更新模型提示。
4. 前端识别展示工具记录，在对应工具 segment 位置渲染图片/文件。
5. 停止自动展示带 ID 的新 artifact，保留无 ID 历史数据兼容。
6. 增加 Rust 与前端单元测试。
7. 运行 typecheck、lint、cargo check、相关测试和 diff check。

## Risk Points

- 流式阶段工具记录与 segment 到达顺序不同，缺失记录必须安全兜底。
- timeline 分组不能把展示工具折叠进普通工具组。
- 历史 artifact 不得因新增 ID 字段消失。
- Markdown 仍需拿到全量 artifact 才能兼容旧图片引用。


## Completion

- [x] Artifact ID contract and Kivio-owned ID assignment
- [x] Native `present_artifacts` registration and structured result
- [x] Model-facing artifact hints and workbench prompt updates
- [x] Timeline-positioned frontend rendering
- [x] Hidden-by-default behavior with legacy no-ID compatibility
- [x] Native-only trust boundary and MCP spoof prevention
- [x] Rust and frontend regression tests
- [x] Typecheck, lint, cargo check, focused tests, and diff check
