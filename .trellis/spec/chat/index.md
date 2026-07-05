# Chat 子系统 Code-Specs

> Chat / Agent 运行时的跨层契约（Rust `src-tauri/src/chat/` ↔ 前端 `src/chat/`）。

## Specs

| Spec | 内容 | 来源 |
|------|------|------|
| [压缩契约](./compaction-contracts.md) | `chat-compaction` 事件配对、boundary 双锚点（切分点 vs 时间线）、`_ui_message_id` runtime→UI 映射 | 07-02-fix-compaction-stuck-and-boundary-mapping |
| [请求形态契约](./request-shape-contracts.md) | 系统提示词前缀稳定性、会话亲和三件套（headers + cacheKey）、`web_search`→`search_web` 保留名 wire 别名、tool_choice/stream usage | 07-02-align-request-shape-and-tool-robustness |
| [连接器门控 Skill 契约](./connector-gated-skills.md) | 连接器就绪才可见的 bundled skill：id 集合 + `skill_connector_satisfied`/`skill_globally_available`/`skill_global_unavailable_error` 三函数、6 处必穿透门控点、vendored 落地约定、dev `target/debug/skills` 快照陷阱 | 07-04-obsidian-connector-skill |
| [Skill 运行时工具契约](./skill-runtime-tool.md) | skill 运行时对模型只暴露单个 `skill` 工具（id `skill__activate`）、`skill_activate→skill` 旧名 alias、读文件走 `read`/脚本走 run_python·run_command、无 `skill_script_allowlist`、chat+kivio_code 两运行时一致 | 07-05-merge-skill-runtime-tools |
| [工具分段↔记录双向对账](./tool-segment-record-reconcile.md) | 工具分段与记录必须双向齐全；孤立分段(有分段无记录)合成 Cancelled 占位记录消除「工具记录缺失」；必接 build_assistant_message / persist_partial / chat_get_conversation 三处 | 07-05-fix-orphan-tool-segment |
