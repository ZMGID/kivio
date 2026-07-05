# 对话分支（分叉成新对话）契约

> 来源：07-05-conversation-branch（方案 B）。

## 背景

对话是严格线性 `Vec<ChatMessage>`（`chat/types.rs`），一对话一个 `conv_<id>.json`。「对话分支」= 在某条消息处把该消息及其之前的消息复制进一个**全新独立对话**（`chat_fork_conversation`），之后各自续聊。源对话**只读、完全不变**。不做会话内消息树 / 内联版本切换。

## 契约

### 分支消息组装（`build_fork_messages`，纯函数）

- 取 `messages[0..=anchor_idx]`（含锚点）。
- 锚点属于多模型多答组（`group_id = Some(g)`）时：**只保留锚点那一列**，移除前缀内其余同组兄弟列，并把锚点 `group_id` 置 `None`（转普通单答）。新对话该轮必须是干净线性单答，不得残留半个多答组。
- 保留原 message id（跨对话无需唯一；`group_selections` 引用因此仍有效）。

### `group_selections` 清理

分叉后仅保留「其选中 `message_id` 仍存在于新 messages、且该消息仍带 `group_id`」的条目。被折叠的组（锚点已去 group_id）与指向已删消息的条目必须淘汰，否则 `group_answer_excluded_from_context` 会把残余答案排除出上下文。

### 附件 / 产物深拷（红线）

附件与图片 artifact 的 `path` 是**相对源对话附件目录的裸文件名**（`attachments.rs`），且删对话会 `remove_dir_all` 该目录。分叉**必须**把被复制消息引用的附件 + 消息级 artifact + 各 `tool_call` artifact 文件按裸文件名深拷进新对话附件目录（`copy_forked_conversation_files`），否则源对话删除后分支附件失效。拒绝含路径分隔符的 `path`（防越目录）；缺文件容错跳过（记 warning，不阻断分叉）。

**已知限制**：沙箱导出的生成文件（`~/Kivio/outputs/<源id>/`，绝对路径引用）**不深拷**——分支中此类 artifact 仍指向源对话 outputs 目录，源在则可用、被删则失效。

### 返回前对账（必接）

`chat_fork_conversation` 直接返回会话（绕过 `chat_get_conversation`），因此返回前必须自行跑 `reconcile_conversation_orphan_tool_segments`，且**在 `strip_transcripts_for_frontend` 之前**（strip 清空已完成消息的 `api_messages`，而回捞工具名依赖它）。见 [[tool-segment-record-reconcile]]。

### 继承与重置

新对话继承源的会话级配置：`provider_id/model`、assistant 快照、`project_id/folder/set_id`、`knowledge_base_ids`、`thinking_level`、`reply_models`、`agent_runtime`、`active_skill_id`。重置：`pinned=false`、`context_state`/`agent_todo_state`/`agent_plan_state` 归默认。标题 = 源标题截断后 + `（分支）`（先给后缀留位，保证 40 字上限内后缀始终可见）。

### `forked_from` 字段（跨层，加字段守则）

- `Conversation` 与 `ConversationListItem` 均新增 `forked_from: Option<ForkOrigin>`，`#[serde(default, skip_serializing_if = "Option::is_none")]`——旧对话 JSON 缺字段正常反序列化。
- 前端 `types.ts` 同步可选 `forked_from`（snake/camel 兼容），会话顶部渲染「分叉自 <源标题>」面包屑，点击 `handleSelectConversation(源id)` 回跳。
- 前端入口：user 气泡动作条与 `AssistantMessageMeta` 各加 `GitBranch`「分支」按钮，沿 `onForkMessage` 透传链（`MessageList`→`MessageBubble`/`MessageGroup`）；生成在飞（`streaming || streamFrozen`）时与重生成一致收起入口。
