# 设计：对话分支（方案 B：分叉成新对话）

## 架构与边界

一条新增后端命令 `chat_fork_conversation` 完成「复制前缀 → 深拷附件 → 建新对话文件」，返回完整 `Conversation`；前端拿到后像打开对话一样应用并刷新侧栏。不触碰 agent loop、上下文拼装、压缩、重放等任何线性遍历逻辑——分叉只是「读源对话 + 生成一个独立的新对话文件」，与现有编辑/重生成完全解耦。

## 数据契约

### `ForkOrigin`（新增，`types.rs`）
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForkOrigin {
    pub conversation_id: String, // 源对话 id
    pub message_id: String,      // 分叉锚点消息 id
    pub title: String,           // 分叉时的源标题快照（源改名/删除不影响面包屑显示）
}
```
- `Conversation` 增字段：`#[serde(default, skip_serializing_if = "Option::is_none")] pub forked_from: Option<ForkOrigin>`。
- `ConversationListItem` 增同名可选字段并在 `From<&Conversation>` 里带上（侧栏可选用；本期至少保证不丢）。
- serde default ⇒ 旧对话 JSON 无此字段正常反序列化（AC6）。前端 `types.ts` 的 `Conversation` 加可选 `forked_from`。

### 命令签名
```rust
#[tauri::command]
async fn chat_fork_conversation(
    app: AppHandle, state: State<AppState>,
    conversation_id: String, message_id: String,
) -> Result<serde_json::Value, String> // { success, conversation }
```

## 核心逻辑（纯函数 + IO 分离）

### 1. 组装分支消息 `build_fork_messages`（纯函数，可单测）
输入源 `messages: &[ChatMessage]`、`anchor_idx`；输出 `Vec<ChatMessage>`：
1. `prefix = messages[0..=anchor_idx].to_vec()`。
2. 若锚点 `group_id = Some(g)`（多模型多答的某列，决策：只留选中列）：
   - 从 `prefix` 中移除所有 `group_id == Some(g)` 且 `id != anchor.id` 的消息（即该组更早的兄弟列；组内更晚的列已被切片排除）。
   - 把锚点那条的 `group_id` 置 `None`，转为普通单答。
3. 保留原 message id（跨对话无需唯一，`group_selections` 引用因此仍有效）。

### 2. 清理 `group_selections`
新对话 `group_selections = 源.group_selections`，仅保留「其选中的 message_id 仍存在于新 messages、且该 message 仍带 group_id」的条目（被折叠的组会因锚点去 group_id 而自动淘汰）。复用 `chat_regenerate_message` 里同款「retain 指向存在消息」的做法。

### 3. 深拷被引用的对话目录文件（R4，AC2）
附件与图片 artifact 的 `path` 都是相对源对话附件目录的**裸文件名**。对新 messages 中每条：
- 每个 `attachment.path`：`copy(src_attach_dir/path → new_attach_dir/path)`（同名，路径保持有效）。
- 每个 artifact（`message.artifacts` + 各 `tool_call.artifacts`）若 `path` 非空：同样按裸文件名拷贝。
- 用现成 `conversation_attachments_dir(app, id)` 取两侧目录；缺文件容错跳过（记 warning，不阻断分叉）。
- **不拷** `~/Kivio/outputs/<id>/` 沙箱导出（见「限制」）。

### 4. 建新对话
- `id = conv_<uuid>`；`title = truncate(源.title + "（分支）", 上限)`。
- `messages = build_fork_messages(...)`（且已完成附件深拷）。
- 继承：`provider_id / model / assistant_id / assistant_snapshot / project_id / folder / set_id / knowledge_base_ids / thinking_level / reply_models / agent_runtime`。
- 重置：`pinned=false`、`context_state=default`、`agent_todo_state=default`、`agent_plan_state=default`、`active_skill_id` 沿用源、`created_at=updated_at=now`。
- `forked_from = Some(ForkOrigin { 源 id, anchor message_id, 源 title })`。
- `save_conversation(app, &new)`（落文件 + 入索引；随后 `strip_transcripts_for_frontend` 再返回）。

不复用空白对话（`find_reusable_blank_conversation`）——分叉必须落在全新容器。

## 前端数据流

`Chat.handleForkMessage(messageId)` → `chatApi.forkConversation(convId, messageId)` → 得到新 `Conversation` → `applyConversation(new)` + `currentConversationIdRef` 更新 + `syncConversationRoute` + `refreshSidebar()`（照搬 `handleSelectConversation` 的应用路径）。

回调透传链（与 `onRegenerateMessage` 平行新增 `onForkMessage?: (messageId) => Promise<void>`）：
`Chat.tsx` → `MessageList` → `MessageBubble`（user 气泡动作条 + assistant 经 `AssistantMessageMeta`）/ `MessageGroup`（每列即某 assistant 消息，透传其 id）。

UI：
- 入口：`GitBranch`（lucide）图标按钮，tooltip「分支」。加进 user 气泡悬浮动作条（复制/编辑/删除旁）与 `AssistantMessageMeta` 动作条（复制/编辑/重生成/删除旁）。
- 生成在飞（`streaming || streamFrozen`）时与重生成一致收起入口（读的是持久态，避免与在飞 run 竞态）。
- 面包屑：会话顶部（MessageList 上方细条）渲染 `forked_from` 时显示「⑂ 分叉自 <title>」，点击 `handleSelectConversation(forked_from.conversation_id)` 跳回源对话。

## 兼容与迁移

- 纯增量字段，全部 serde default；无存储迁移。旧对话加载零影响（AC6）。
- 源对话完全只读，分叉不改动它（AC1）。

## 限制（已知，非阻断）

- 沙箱导出的**生成文件**（`~/Kivio/outputs/<源id>/`，非图片 artifact）不随分叉深拷：分支中这类 artifact 仍指向源对话的 outputs 目录，源对话在时可用、被删则失效。附件与图片 artifact 不受此限（已深拷）。后续可作为增强项。

## 权衡

- 保留原 message id 而非重新生成：省去 `group_selections`/引用重映射，跨对话 id 不要求唯一，最简且正确。
- 沙箱导出不拷：控制本期范围；相比附件使用频率低，且路径为绝对路径、拷贝需改写 artifact path，成本收益不划算。

## 回滚

改动集中在：`types.rs`(+字段/结构)、`commands.rs`(+1 命令 +纯函数)、`lib.rs`(注册命令)、`api.ts`/`types.ts`/`Chat.tsx`/`MessageList.tsx`/`MessageBubble.tsx`/`AssistantMessageMeta.tsx`/`MessageGroup.tsx`(+回调与按钮)。回滚 = 撤销命令注册 + 移除 UI 入口；新增字段可保留（无害）。
