# Chat 子系统 Code-Specs

> Chat / Agent 运行时的跨层契约（Rust `src-tauri/src/chat/` ↔ 前端 `src/chat/`）。

## Specs

| Spec | 内容 | 来源 |
|------|------|------|
| [压缩契约](./compaction-contracts.md) | `chat-compaction` 事件配对、boundary 双锚点（切分点 vs 时间线）、`_ui_message_id` runtime→UI 映射；链式摘要合并（`PERSISTED_SUMMARY_PREFIX` 共用）、估算口径（`estimate_value_tokens` 图片记0 / 展开工具转录）、多答排除臂过滤、取消≠失败；**真实用量锚点**（provider 实报 usage 为主口径 + chars 兜底、家族消歧防双算、`max` 保守下限、provider切换/压缩边界后失效、`provider_reported` 跨层标记） | 07-02-fix-compaction-stuck-and-boundary-mapping · 07-06-compaction-correctness-fixes · 07-06-context-token-ground-truth |
| [请求形态契约](./request-shape-contracts.md) | 系统提示词前缀稳定性、会话亲和三件套（headers + cacheKey）、`web_search`→`search_web` 保留名 wire 别名、tool_choice/stream usage | 07-02-align-request-shape-and-tool-robustness |
| [连接器门控 Skill 契约](./connector-gated-skills.md) | 连接器就绪才可见的 bundled skill：id 集合 + `skill_connector_satisfied`/`skill_globally_available`/`skill_global_unavailable_error` 三函数、6 处必穿透门控点、vendored 落地约定、dev `target/debug/skills` 快照陷阱 | 07-04-obsidian-connector-skill |
| [Skill 运行时工具契约](./skill-runtime-tool.md) | skill 运行时对模型只暴露单个 `skill` 工具（id `skill__activate`）、`skill_activate→skill` 旧名 alias、读文件走 `read`/脚本走 run_python·run_command、无 `skill_script_allowlist`、chat+kivio_code 两运行时一致 | 07-05-merge-skill-runtime-tools |
| [工具分段↔记录双向对账](./tool-segment-record-reconcile.md) | 工具分段与记录必须双向齐全；孤立分段(有分段无记录)合成 Cancelled 占位记录消除「工具记录缺失」；必接 build_assistant_message / persist_partial / chat_get_conversation 三处 | 07-05-fix-orphan-tool-segment |
| [对话分支（分叉成新对话）](./conversation-fork.md) | 方案 B：复制 messages[0..=锚点]→新对话；多答组折叠为选中列去 group_id；附件/图片 artifact 按裸文件名深拷（沙箱导出不拷）；返回前 reconcile 须在 strip 前；forked_from 加字段+面包屑回跳 | 07-05-conversation-branch |
| [MCP 工具结果图片直达模型](./mcp-image-feedback.md) | `attach_image_artifacts_for_model`：vision 主模型经 `follow_up_user_messages` 直喂 / 非 vision 走辅助视觉审查向分析；护栏 4图/8MB；已喂模型的图清 artifacts 不进聊天画廊；辅助视觉提示词必须审查向；插件适配原则（通用能力进 runtime，插件 hint 只留专属约束） | 07-10-plugin-runtime-robustness |
| [MCP session concurrency](./mcp-session-concurrency.md) | Separate lifecycle locking from stdio per-request waits; multiplex by JSON-RPC id; serialize stdin writes only; ambiguous timeouts are unknown-outcome and never automatically retried; reconnect only dead transports | 07-10-plugin-runtime-robustness |
