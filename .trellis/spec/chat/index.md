# Chat 子系统 Code-Specs

> Chat / Agent 运行时的跨层契约（Rust `src-tauri/src/chat/` ↔ 前端 `src/chat/`）。

## Specs

| Spec | 内容 | 来源 |
|------|------|------|
| [压缩契约](./compaction-contracts.md) | `chat-compaction` 事件配对、boundary 双锚点（切分点 vs 时间线）、`_ui_message_id` runtime→UI 映射 | 07-02-fix-compaction-stuck-and-boundary-mapping |
| [请求形态契约](./request-shape-contracts.md) | 系统提示词前缀稳定性、会话亲和三件套（headers + cacheKey）、`web_search`→`search_web` 保留名 wire 别名、tool_choice/stream usage | 07-02-align-request-shape-and-tool-robustness |
