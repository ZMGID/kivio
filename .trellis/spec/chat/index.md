# Chat 子系统 Code-Specs

> Chat / Agent 运行时的跨层契约（Rust `src-tauri/src/chat/` ↔ 前端 `src/chat/`）。

## Specs

| Spec | 内容 | 来源 |
|------|------|------|
| [压缩契约](./compaction-contracts.md) | `chat-compaction` 事件配对、boundary 双锚点（切分点 vs 时间线）、`_ui_message_id` runtime→UI 映射 | 07-02-fix-compaction-stuck-and-boundary-mapping |
| [请求形态契约](./request-shape-contracts.md) | 系统提示词前缀稳定性、会话亲和三件套（headers + cacheKey）、`web_search`→`search_web` 保留名 wire 别名、tool_choice/stream usage | 07-02-align-request-shape-and-tool-robustness |
| [连接器门控 Skill 契约](./connector-gated-skills.md) | 连接器就绪才可见的 bundled skill：id 集合 + `skill_connector_satisfied`/`skill_globally_available`/`skill_global_unavailable_error` 三函数、6 处必穿透门控点、vendored 落地约定、dev `target/debug/skills` 快照陷阱 | 07-04-obsidian-connector-skill |
